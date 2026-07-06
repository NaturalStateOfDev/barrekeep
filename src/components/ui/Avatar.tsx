import { avatarTint } from "../../lib/avatar";
import { initials } from "../../lib/dates";

interface Props {
  name: string | null;
  size?: number;
}

export function Avatar({ name, size = 22 }: Props) {
  return (
    <span
      className="avatar"
      title={name ?? undefined}
      style={{
        width: size,
        height: size,
        fontSize: Math.round(size * 0.42),
        background: avatarTint(name),
      }}
    >
      {initials(name)}
    </span>
  );
}
