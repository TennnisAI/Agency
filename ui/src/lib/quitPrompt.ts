// The quit dialog's words, kept apart from App so the copy is testable.

/** What the quit dialog says with `running` sessions in the daemon. Two ways
 *  out when anything is running (AGE-210): leave the sessions to the daemon,
 *  or stop them. */
export function quitBody(running: number): string {
  if (running === 0) return "All agents are idle; nothing will be interrupted.";
  const one = running === 1;
  const them = one ? "it" : "them";
  return `${running} session${one ? " is" : "s are"} running. Leave ${them} running and `
    + `${one ? "it carries" : "they carry"} on while Agency is closed, without notifications, `
    + `until you reopen it. Stop ${them} and agents resume where they left off next time.`;
}
