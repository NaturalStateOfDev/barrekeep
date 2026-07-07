// Per-teacher avatar tints, harmonized with the class-chip palette
// (Barre & Bloom). The tint is a stable hash of the display name so a
// teacher keeps their color across pulls and proposals.

export const AVATAR_TINTS = [
  "#8d7aa0", // plum-grey
  "#c77e93", // berry
  "#7fa27f", // sage
  "#cf9a54", // ochre
  "#6aa3aa", // teal
  "#9a76ad", // violet
];

export function avatarTint(name: string | null): string {
  const s = name ?? "";
  let hash = 0;
  for (let i = 0; i < s.length; i++) {
    hash = (hash * 31 + s.charCodeAt(i)) | 0;
  }
  return AVATAR_TINTS[Math.abs(hash) % AVATAR_TINTS.length];
}
