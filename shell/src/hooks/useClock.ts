import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";

export function useClock() {
  const setTime = useSystemStore((s) => s.setTime);

  useEffect(() => {
    let prevTime = "";
    function update() {
      const now = new Date();
      const formatted = now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false });
      if (formatted !== prevTime) {
        prevTime = formatted;
        setTime(formatted);
      }
    }
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [setTime]);
}
