export default function Toggle({
  checked,
  onChange,
}: {
  checked: boolean;
  onChange: (next: boolean) => void;
}) {
  return (
    <span className={`toggle ${checked ? "on" : ""}`}>
      <input
        type="checkbox"
        checked={checked}
        onChange={(e) => onChange(e.target.checked)}
      />
      <span className="toggle-track">
        <span className="toggle-thumb" />
      </span>
    </span>
  );
}
