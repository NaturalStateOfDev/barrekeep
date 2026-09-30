// The user's time-display preference (Settings → Display → "Use 24-hour
// time"), stored in app_settings under `time_format` = "12h" | "24h"
// (default 12h). Display-only — see the time helpers in ./dates.

import { createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode } from "react";
import { api } from "./api";
import {
  DEFAULT_TIME_FORMAT,
  formatTime,
  formatTimeRange,
  formatTimeShort,
  formatTimestamp,
  type TimeFormat,
} from "./dates";

export const TIME_FORMAT_SETTING = "time_format";

export function parseTimeFormat(v: string | null | undefined): TimeFormat {
  return v === "24h" ? "24h" : DEFAULT_TIME_FORMAT;
}

interface Ctx {
  fmt: TimeFormat;
  setFormat: (fmt: TimeFormat) => Promise<void>;
}

const TimeFormatContext = createContext<Ctx>({
  fmt: DEFAULT_TIME_FORMAT,
  setFormat: async () => {},
});

export function TimeFormatProvider({ children }: { children: ReactNode }) {
  const [fmt, setFmt] = useState<TimeFormat>(DEFAULT_TIME_FORMAT);

  useEffect(() => {
    api.getAppSetting(TIME_FORMAT_SETTING)
      .then((v) => setFmt(parseTimeFormat(v)))
      .catch(() => { /* keep the default */ });
  }, []);

  const setFormat = useCallback(async (next: TimeFormat) => {
    setFmt(next);
    await api.setAppSetting(TIME_FORMAT_SETTING, next);
  }, []);

  const value = useMemo(() => ({ fmt, setFormat }), [fmt, setFormat]);
  return <TimeFormatContext.Provider value={value}>{children}</TimeFormatContext.Provider>;
}

/** Formatters bound to the user's preference. */
export function useTimeFormat() {
  const { fmt, setFormat } = useContext(TimeFormatContext);
  return useMemo(
    () => ({
      fmt,
      setFormat,
      /** "9:45 AM" / "09:45" */
      time: (hhmm: string) => formatTime(hhmm, fmt),
      /** "9:45a" / "09:45" — tight spaces only */
      timeShort: (hhmm: string) => formatTimeShort(hhmm, fmt),
      range: (start: string, end: string, compact = false) => formatTimeRange(start, end, fmt, compact),
      timestamp: (ts: string) => formatTimestamp(ts, fmt),
    }),
    [fmt, setFormat],
  );
}
