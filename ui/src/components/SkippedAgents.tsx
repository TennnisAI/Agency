import { CustomAgents } from "../api";

// Agent files that couldn't be read. An agent missing from the menu with the
// reason only in a log looks like the app lost it.
export default function SkippedAgents({ skipped }: { skipped: CustomAgents["skipped"] }) {
  return (
    <>
      {skipped.map((s) => (
        <div key={s.file} className="git-error">Couldn't load {s.file}: {s.reason}</div>
      ))}
    </>
  );
}
