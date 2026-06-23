/** Join a child name onto a worktree-relative parent path. The root is "". */
export function joinPath(parent: string, name: string): string {
  return parent ? `${parent}/${name}` : name;
}
