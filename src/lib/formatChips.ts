export interface Chip {
  label: string;
  token: string;
}

const TABLE: Record<string, Chip> = {
  Classic:     { label: "Classic",     token: "--chip-slate" },
  Empower:     { label: "Empower",     token: "--chip-terracotta" },
  Define:      { label: "Define",      token: "--chip-sage" },
  Reform:      { label: "Reform",      token: "--chip-ochre" },
  Foundations: { label: "Foundations", token: "--chip-plum" },
  Focus:       { label: "Focus",       token: "--chip-accent" },
};

const FALLBACK: Chip = { label: "?", token: "--chip-default" };

export function chipFor(className: string): Chip {
  return TABLE[className] ?? { label: className, token: FALLBACK.token };
}
