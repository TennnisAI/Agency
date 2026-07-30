export default function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  // A <label>, not a <span>: WebKit keeps an absolutely-positioned checkbox at
  // its intrinsic ~12px size (it doesn't stretch to inset:0 like Chromium), so
  // clicks on the visible track used to miss the input entirely in WKWebView —
  // the label forwards them natively in every engine.
  return (
    <label className={`toggle ${checked ? "on" : ""}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle-track">
        <span className="toggle-thumb" />
      </span>
    </label>
  );
}
