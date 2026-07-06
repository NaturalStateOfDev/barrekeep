export function LoadingBlock({ label = "Working…" }: { label?: string }) {
  return (
    <div className="bk-loading" role="status">
      <span className="bk-spinner" />
      <div>{label}</div>
    </div>
  );
}
