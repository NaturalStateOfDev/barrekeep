interface Props<T extends string> {
  tabs: readonly T[];
  value: T;
  onChange: (next: T) => void;
}

export function Tabs<T extends string>({ tabs, value, onChange }: Props<T>) {
  return (
    <div className="bk-tabs">
      {tabs.map((t) => (
        <button key={t} onClick={() => onChange(t)} className={t === value ? "active" : ""}>
          {t}
        </button>
      ))}
    </div>
  );
}
