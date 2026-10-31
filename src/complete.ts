import sprites from "./sprites.json";

/** One completion: `value` replaces the whole input when accepted. */
export type Suggestion = { label: string; hint: string; value: string; final: boolean };

type Arg = { name: string; hint?: string; values?: string[] };

/** Mirrors the commands in src-tauri/src/settings.rs. */
const COMMANDS: { cmd: string; hint: string; args?: Arg[]; keys?: Arg[] }[] = [
  {
    cmd: "settings",
    hint: "show or change settings",
    keys: [
      { name: "skin", hint: "how i look", values: Object.keys(sprites.skins) },
      { name: "name", hint: "what to call me", values: [] },
      { name: "model", hint: "claude model", values: ["haiku", "sonnet", "opus"] },
      { name: "voice", hint: "read answers out loud", values: ["on", "off"] },
      { name: "motivate", hint: "minutes between pep talks", values: ["off", "15", "30", "45", "60", "90"] },
      { name: "speed", hint: "stat decay multiplier", values: ["0", "0.5", "1", "2", "5"] },
      { name: "ontop", hint: "always on top", values: ["on", "off"] },
      { name: "limit", hint: "chats per hour", values: ["10", "30", "60", "100"] },
    ],
  },
  { cmd: "remember", hint: "something i should always remember" },
  { cmd: "memories", hint: "what i remember about you" },
  { cmd: "forget", hint: "erase everything i remember" },
  { cmd: "motivate", hint: "pep talk right now" },
  { cmd: "stats", hint: "exact stat numbers" },
  { cmd: "doctor", hint: "check claude is set up" },
  { cmd: "personality", hint: "edit my personality file", args: [{ name: "reset", hint: "restore the default" }] },
  { cmd: "clear", hint: "forget our conversation" },
  { cmd: "reset", hint: "hatch a new pet", args: [{ name: "confirm", hint: "yes, really" }] },
  { cmd: "help", hint: "list commands" },
  { cmd: "bye", hint: "say goodbye and close the app" },
];

const starts = (s: string, prefix: string) => s.toLowerCase().startsWith(prefix.toLowerCase());

/** Suggestions for what's typed so far. Empty unless the input starts with `/`. */
export function suggest(input: string): Suggestion[] {
  if (!input.startsWith("/")) return [];
  const parts = input.slice(1).split(/\s+/);
  const [cmdPart, ...rest] = parts;

  if (rest.length === 0) {
    return COMMANDS.filter((c) => starts(c.cmd, cmdPart)).map((c) => ({
      label: `/${c.cmd}`,
      hint: c.hint,
      // Only /settings needs a next part; other arguments are optional (and destructive:
      // `reset`, `confirm`), so completing never sets them up to be picked by Enter.
      value: `/${c.cmd}${c.keys ? " " : ""}`,
      final: !c.keys,
    }));
  }

  const command = COMMANDS.find((c) => c.cmd === cmdPart.toLowerCase());
  if (!command) return [];

  if (command.args) {
    if (rest.length > 1) return [];
    return command.args
      .filter((a) => starts(a.name, rest[0]))
      .map((a) => ({ label: a.name, hint: a.hint ?? "", value: `/${command.cmd} ${a.name}`, final: true }));
  }

  if (command.keys) {
    const [keyPart, ...valueParts] = rest;
    if (valueParts.length === 0) {
      return command.keys
        .filter((k) => starts(k.name, keyPart))
        .map((k) => ({ label: k.name, hint: k.hint ?? "", value: `/${command.cmd} ${k.name} `, final: false }));
    }
    const key = command.keys.find((k) => k.name === keyPart.toLowerCase());
    if (!key?.values || valueParts.length > 1) return [];
    return key.values
      .filter((v) => starts(v, valueParts[0]))
      .map((v) => ({ label: v, hint: "", value: `/${command.cmd} ${key.name} ${v}`, final: true }));
  }
  return [];
}
