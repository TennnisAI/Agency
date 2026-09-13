// The model a spawn launches on, from what its caller said (see SpawnOpts):
// a string is that model, `null` is the agent's own default, and `undefined`
// means the caller had no picker in front of the user, so the model the agent
// last ran on is repeated.
//
// Every spawn path has to read it the same way. AGE-236: an issue's ▶ button
// passes no options, and issue dispatch read that as `opts?.model ?? null`, the
// agent's default. So AGE-235 launched with no `--model` and came up on Fable,
// claude's own saved default, straight after the last run had been started on
// opus; that launch then also cleared opus as the remembered model.
export function launchModel(
  model: string | null | undefined,
  remembered: () => Promise<string | null>,
): Promise<string | null> {
  return model !== undefined ? Promise.resolve(model) : remembered();
}
