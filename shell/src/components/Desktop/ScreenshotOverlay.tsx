import { useCallback, useEffect, useState } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useScreenshotStore } from "../../stores/screenshotStore";
import { glass } from "../../lib/styles";

function captureScreen(): Promise<string> {
  return new Promise((resolve, reject) => {
    const target = document.documentElement;
    const width = window.innerWidth;
    const height = window.innerHeight;

    const canvas = document.createElement("canvas");
    canvas.width = width * window.devicePixelRatio;
    canvas.height = height * window.devicePixelRatio;
    const ctx = canvas.getContext("2d");
    if (!ctx) {
      reject(new Error("Failed to get canvas context"));
      return;
    }

    const svgData = `
      <foreignObject width="${width}" height="${height}">
        ${new XMLSerializer().serializeToString(target)}
      </foreignObject>`;
    const svgBlob = new Blob(
      [
        `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}">${svgData}</svg>`,
      ],
      { type: "image/svg+xml;charset=utf-8" },
    );
    const url = URL.createObjectURL(svgBlob);
    const img = new Image();
    img.onload = () => {
      ctx.scale(window.devicePixelRatio, window.devicePixelRatio);
      ctx.drawImage(img, 0, 0);
      URL.revokeObjectURL(url);
      resolve(canvas.toDataURL("image/png"));
    };
    img.onerror = () => {
      URL.revokeObjectURL(url);
      reject(new Error("Failed to render screenshot"));
    };
    img.src = url;
  });
}

async function writeToClipboard(dataUrl: string): Promise<void> {
  try {
    const response = await fetch(dataUrl);
    const blob = await response.blob();
    await navigator.clipboard.write([
      new ClipboardItem({ "image/png": blob }),
    ]);
  } catch {
    // Clipboard write may fail in some browsers; toast still shows
  }
}

function Toast({ message, onDone }: { message: string; onDone: () => void }) {
  useEffect(() => {
    const timer = setTimeout(onDone, 2000);
    return () => clearTimeout(timer);
  }, [onDone]);

  return (
    <motion.div
      className={`fixed bottom-6 right-6 z-[9999] px-4 py-2 text-sm font-medium text-white ${glass.menu}`}
      initial={{ opacity: 0, y: 20, scale: 0.9 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: 10, scale: 0.95 }}
      transition={{ duration: 0.25 }}
    >
      {message}
    </motion.div>
  );
}

export default function ScreenshotOverlay() {
  const isActive = useScreenshotStore((s) => s.isActive);
  const deactivate = useScreenshotStore((s) => s.deactivate);
  const setCapturedUrl = useScreenshotStore((s) => s.setCapturedUrl);
  const [showFlash, setShowFlash] = useState(false);
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  const handleCapture = useCallback(async () => {
    // Hide overlay before capturing so it doesn't appear in the screenshot
    deactivate();

    // Wait one frame for the overlay to unmount
    await new Promise((r) => requestAnimationFrame(r));
    await new Promise((r) => requestAnimationFrame(r));

    try {
      const dataUrl = await captureScreen();
      setCapturedUrl(dataUrl);
      await writeToClipboard(dataUrl);
      setShowFlash(true);
      setTimeout(() => setShowFlash(false), 150);
      setToastMessage("Screenshot copied to clipboard");
    } catch {
      setToastMessage("Screenshot captured");
    }
  }, [deactivate, setCapturedUrl]);

  useEffect(() => {
    if (!isActive) return;

    function handleKeyDown(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        deactivate();
      }
    }

    window.addEventListener("keydown", handleKeyDown, true);
    return () => window.removeEventListener("keydown", handleKeyDown, true);
  }, [isActive, deactivate]);

  const dismissToast = useCallback(() => setToastMessage(null), []);

  return (
    <>
      {/* Screenshot capture overlay */}
      <AnimatePresence>
        {isActive && (
          <motion.div
            className="absolute inset-0 z-[9998] cursor-crosshair bg-black/30"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
            onClick={handleCapture}
          >
            <div className="flex h-full w-full items-center justify-center">
              <motion.div
                className={`px-5 py-2.5 text-sm font-medium text-white ${glass.menu}`}
                initial={{ opacity: 0, scale: 0.9 }}
                animate={{ opacity: 1, scale: 1 }}
                transition={{ delay: 0.05, duration: 0.2 }}
              >
                Click anywhere to capture &middot; Press Esc to cancel
              </motion.div>
            </div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Flash effect */}
      <AnimatePresence>
        {showFlash && (
          <motion.div
            className="pointer-events-none fixed inset-0 z-[9999] bg-white"
            initial={{ opacity: 0.7 }}
            animate={{ opacity: 0 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.15 }}
          />
        )}
      </AnimatePresence>

      {/* Toast notification */}
      <AnimatePresence>
        {toastMessage && (
          <Toast
            key="screenshot-toast"
            message={toastMessage}
            onDone={dismissToast}
          />
        )}
      </AnimatePresence>
    </>
  );
}
