import { AnimatePresence } from "framer-motion";
import { useSessionStore } from "./stores/sessionStore";
import { useClock } from "./hooks/useClock";
import { useSystemInfo } from "./hooks/useSystemInfo";
import LoginScreen from "./components/LoginScreen/LoginScreen";
import Desktop from "./components/Desktop/Desktop";

export default function App() {
  const isLoggedIn = useSessionStore((s) => s.isLoggedIn);
  useClock();
  useSystemInfo();

  return (
    <AnimatePresence mode="wait">
      {isLoggedIn ? (
        <Desktop key="desktop" />
      ) : (
        <LoginScreen key="login" />
      )}
    </AnimatePresence>
  );
}
