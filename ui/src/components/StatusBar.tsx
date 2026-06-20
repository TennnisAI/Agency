export default function StatusBar({ projectName }: { projectName: string | null }) {
  return (
    <footer className="statusbar">
      <span>{projectName ?? "no project"}</span>
      <span className="kbd-hints">⌘N new · ⌘G source · ⌘↵ approve</span>
    </footer>
  );
}
