//! SSRF-guarded fetch proxy.
//!
//! The browser can't fetch arbitrary cross-origin pages (CORS) and recipe sites
//! block scrapers, so the frontend asks this endpoint to fetch a URL
//! server-side and hand back the raw bytes (which it then parses via
//! recipe-core WASM). Because this fetches attacker-influenced URLs, it is an
//! SSRF surface: the guard below rejects any target that resolves to a
//! non-public address, and it does so at the DNS layer so it also covers
//! redirect hops (no TOCTOU / DNS-rebinding gap between validation and connect).

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::{extract::State, Json};
use reqwest::dns::{Addrs, Name, Resolve, Resolving};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Cap on the bytes we read from an upstream response (recipe pages are small).
const MAX_BODY_BYTES: usize = 3 * 1024 * 1024;
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REDIRECTS: usize = 4;

#[derive(Clone)]
pub struct AppState {
    pub http: reqwest::Client,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .dns_resolver(Arc::new(GuardedResolver))
            .redirect(reqwest::redirect::Policy::limited(MAX_REDIRECTS))
            .timeout(FETCH_TIMEOUT)
            .user_agent(concat!(
                "recipes-proxy/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/thedavidmeister/recipes)"
            ))
            .build()?;
        Ok(Self { http })
    }
}

#[derive(Debug, Deserialize)]
pub struct FetchRequest {
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct FetchResponse {
    /// The URL actually fetched (after any redirects).
    pub final_url: String,
    pub content_type: Option<String>,
    pub body: String,
}

/// `POST /api/fetch` — fetch `url` server-side and return its body.
pub async fn fetch(
    State(state): State<AppState>,
    Json(req): Json<FetchRequest>,
) -> Result<Json<FetchResponse>, AppError> {
    let url = reqwest::Url::parse(req.url.trim())
        .map_err(|_| AppError::BadRequest("invalid url".into()))?;

    if !matches!(url.scheme(), "http" | "https") {
        return Err(AppError::BadRequest("only http(s) urls are allowed".into()));
    }

    // A literal-IP host never hits the resolver, so validate it here too.
    match url.host() {
        None => return Err(AppError::BadRequest("url has no host".into())),
        Some(url::Host::Ipv4(ip)) if !is_public_ip(IpAddr::V4(ip)) => {
            return Err(AppError::Blocked)
        }
        Some(url::Host::Ipv6(ip)) if !is_public_ip(IpAddr::V6(ip)) => {
            return Err(AppError::Blocked)
        }
        _ => {}
    }

    let resp = state.http.get(url).send().await?.error_for_status()?;
    let final_url = resp.url().to_string();
    let content_type = resp
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let body = read_capped(resp, MAX_BODY_BYTES).await?;

    Ok(Json(FetchResponse {
        final_url,
        content_type,
        body,
    }))
}

/// Read the response body, failing if it exceeds `max` bytes.
async fn read_capped(resp: reqwest::Response, max: usize) -> Result<String, AppError> {
    use futures_util::StreamExt;

    let mut stream = resp.bytes_stream();
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if buf.len() + chunk.len() > max {
            return Err(AppError::BadRequest(
                "upstream response exceeds size limit".into(),
            ));
        }
        buf.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// A `reqwest` DNS resolver that drops any non-public address before a
/// connection is made. Guards the initial request *and* every redirect hop.
struct GuardedResolver;

impl Resolve for GuardedResolver {
    fn resolve(&self, name: Name) -> Resolving {
        Box::pin(async move {
            let host = name.as_str().to_owned();
            let addrs = tokio::net::lookup_host((host.as_str(), 0)).await?;
            let public: Vec<SocketAddr> = addrs.filter(|sa| is_public_ip(sa.ip())).collect();
            if public.is_empty() {
                return Err(Box::<dyn std::error::Error + Send + Sync>::from(
                    "blocked: host resolves only to non-public addresses",
                ));
            }
            let iter: Addrs = Box::new(public.into_iter());
            Ok(iter)
        })
    }
}

/// Whether `ip` is a routable public address (i.e. NOT loopback, private,
/// link-local — which includes the 169.254.169.254 cloud metadata endpoint —
/// unique-local, unspecified, multicast, etc.).
fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, ..] = v4.octets();
            !(v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_unspecified()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_documentation()
                || a == 0
                // 100.64.0.0/10 carrier-grade NAT
                || (a == 100 && (b & 0xc0) == 0x40))
        }
        IpAddr::V6(v6) => {
            // An IPv4-mapped address (::ffff:a.b.c.d) must be judged as its v4.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return is_public_ip(IpAddr::V4(mapped));
            }
            let seg0 = v6.segments()[0];
            let is_unique_local = (seg0 & 0xfe00) == 0xfc00; // fc00::/7
            let is_link_local = (seg0 & 0xffc0) == 0xfe80; // fe80::/10
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                || is_unique_local
                || is_link_local)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_public_ip;
    use std::net::IpAddr;

    fn public(s: &str) -> bool {
        is_public_ip(s.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn public_addresses_allowed() {
        assert!(public("8.8.8.8"));
        assert!(public("1.1.1.1"));
        assert!(public("93.184.216.34")); // example.com
        assert!(public("2606:2800:220:1:248:1893:25c8:1946"));
    }

    #[test]
    fn private_and_local_blocked() {
        for ip in [
            "127.0.0.1",              // loopback
            "10.0.0.1",               // private
            "192.168.1.1",            // private
            "172.16.0.1",             // private
            "169.254.169.254",        // link-local / cloud metadata
            "0.0.0.0",                // unspecified
            "100.64.0.1",             // CGNAT
            "224.0.0.1",              // multicast
            "::1",                    // v6 loopback
            "fc00::1",                // v6 unique-local
            "fe80::1",                // v6 link-local
            "::ffff:127.0.0.1",       // v4-mapped loopback
            "::ffff:169.254.169.254", // v4-mapped metadata
        ] {
            assert!(!public(ip), "{ip} must be blocked");
        }
    }
}
