interface Props {
  targetMonth: string; // "YYYY-MM"
}

export function CalendarHeader({ targetMonth }: Props) {
  const [y, m] = targetMonth.split("-").map(Number);
  const monthNames = ["January","February","March","April","May","June","July","August","September","October","November","December"];
  return (
    <div className="bk-cal-header">
      <h2>{monthNames[m - 1]} {y}</h2>
    </div>
  );
}
