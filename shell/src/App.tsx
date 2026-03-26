import { AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Desktop from "./components/Desktop/Desktop";

export default function App() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  const isLocked = useSessionStore((s) => s.isLocked);
  useClock();
  useSystemInfo();

  const showLogin = !isLoggedIn || isLocked;

  return (
    <AnimatePresence mode="wait">
      {showLogin ? (
        <LoginScreen key="login" />
      ) : (
        <Desktop key="desktop" />
      )}
    </AnimatePresence>
  );
}
