import { useEffect } from "react";
import { useSystemStore } from "../stores/systemStore";

export function useClock() {
  const setTime = useSystemStore((s) => s.setTime);

  useEffect(() => {
    function update() {
      const now = new Date();
      setTime(
        now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", hour12: false }),
      );
    }
    update();
    const id = setInterval(update, 1000);
    return () => clearInterval(id);
  }, [setTime]);
}
