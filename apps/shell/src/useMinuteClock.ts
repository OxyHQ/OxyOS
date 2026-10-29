import { useEffect, useState } from "react";

/**
 * The time a menu bar clock shows: now, updated as each minute begins. One timer
 * for the next minute's start, not a poll, so the clock changes exactly on the
 * minute and wakes nothing in between.
 */
export function useMinuteClock(): Date {
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const untilNextMinute = (from: Date) =>
      60_000 - (from.getSeconds() * 1000 + from.getMilliseconds());
    const tick = () => {
      const current = new Date();
      setNow(current);
      timer = setTimeout(tick, untilNextMinute(current));
    };
    timer = setTimeout(tick, untilNextMinute(new Date()));
    return () => clearTimeout(timer);
  }, []);
  return now;
}
