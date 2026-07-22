<script lang="ts">
  import qrcode from "qrcode-generator";

  /**
   * A QR code for a link — a kitchen's invite (#72), so someone can scan it off your
   * screen with their phone instead of having to get the URL onto that device.
   *
   * Rendered as inline SVG from the module matrix: no image request, no external
   * service, and it inherits the palette (dark modules take `currentColor`) rather
   * than hardcoding black on white — which the design fence forbids anyway, and a
   * dark-on-light token pair scans just as well.
   */
  interface Props {
    /** What the code encodes. */
    value: string;
    /** What scanning it does — the accessible name. */
    label: string;
  }

  let { value, label }: Props = $props();

  /** Modules of light margin around the code. Scanners need a quiet zone to lock on. */
  const QUIET = 2;

  // Type 0 auto-fits the smallest version that holds the value; "M" correction is the
  // usual trade-off for a URL — recovers ~15% while keeping the modules coarse enough
  // to scan off a screen.
  const code = $derived.by(() => {
    const qr = qrcode(0, "M");
    qr.addData(value);
    qr.make();
    const count = qr.getModuleCount();
    let path = "";
    for (let row = 0; row < count; row++) {
      for (let col = 0; col < count; col++) {
        if (qr.isDark(row, col)) path += `M${col} ${row}h1v1h-1z`;
      }
    }
    return { count, path, span: count + QUIET * 2 };
  });
</script>

<svg
  viewBox="{-QUIET} {-QUIET} {code.span} {code.span}"
  role="img"
  aria-label={label}
  class="rounded-card size-48 text-stone-900"
>
  <!-- The quiet zone + ground. Colour comes through `currentColor` on a wrapping
       text utility, the same way the modules do — no raw fill values. -->
  <g class="text-cream-50">
    <rect
      x={-QUIET}
      y={-QUIET}
      width={code.span}
      height={code.span}
      fill="currentColor"
    />
  </g>
  <path d={code.path} fill="currentColor" />
</svg>
