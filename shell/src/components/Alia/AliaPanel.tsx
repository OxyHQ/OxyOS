import { useState, useRef, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useAliaStore } from "../../stores/aliaStore";
import { streamChat } from "../../lib/alia";
import AliaFace from "./AliaFace";
import type { AliaExpression } from "./AliaFace";

export default function AliaPanel() {
  const messages = useAliaStore((s) => s.messages);
  const isStreaming = useAliaStore((s) => s.isStreaming);
  const isListening = useAliaStore((s) => s.isListening);
  const close = useAliaStore((s) => s.close);
  const addMessage = useAliaStore((s) => s.addMessage);
  const appendToLastMessage = useAliaStore((s) => s.appendToLastMessage);
  const setStreaming = useAliaStore((s) => s.setStreaming);
  const toggleListening = useAliaStore((s) => s.toggleListening);
  const clearMessages = useAliaStore((s) => s.clearMessages);

  const [input, setInput] = useState("");
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLTextAreaElement>(null);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  useEffect(() => {
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  const sendMessage = useCallback(async () => {
    const text = input.trim();
    if (!text || isStreaming) return;

    setInput("");
    addMessage("user", text);

    const store = useAliaStore.getState();
    const chatHistory = [...store.messages, { id: "", role: "user" as const, content: text, timestamp: 0 }]
      .filter((m) => m.id !== "greeting")
      .map((m) => ({ role: m.role, content: m.content }));

    addMessage("assistant", "");
    setStreaming(true);

    try {
      for await (const delta of streamChat(chatHistory)) {
        appendToLastMessage(delta);
      }
    } catch (err) {
      appendToLastMessage(
        err instanceof Error ? `\n\n_Error: ${err.message}_` : "\n\n_Something went wrong._"
      );
    } finally {
      setStreaming(false);
    }
  }, [input, isStreaming, addMessage, appendToLastMessage, setStreaming]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  return (
    <motion.div
      initial={{ opacity: 0, y: -8, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -8, scale: 0.97 }}
      transition={{ duration: 0.22, ease: [0.2, 0, 0, 1] }}
      className="fixed top-4 right-4 z-50 flex w-[420px] origin-top-right flex-col overflow-hidden rounded-[20px] border border-white/20 bg-white/12 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]"
      style={{ maxHeight: "calc(100vh - 80px)" }}
    >
      {/* ── Header ── */}
      <div className="flex items-center justify-between border-b border-white/8 px-4 py-3">
        <div className="flex items-center gap-2.5">
          <AliaFace size={28} expression={isStreaming ? "speaking" : isListening ? "listening" : "idle"} />
          <span className="text-[14px] font-semibold text-white/90">Alia</span>
          {isStreaming && (
            <span className="text-[11px] text-white/35">typing...</span>
          )}
        </div>
        <div className="flex items-center gap-1">
          {/* Clear */}
          <button
            onClick={clearMessages}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/30 transition-colors hover:bg-white/8 hover:text-white/55"
            aria-label="Clear chat"
            title="Clear chat"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          </button>
          {/* Minimize */}
          <button
            onClick={close}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/30 transition-colors hover:bg-white/8 hover:text-white/55"
            aria-label="Minimize"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <line x1="5" y1="12" x2="19" y2="12" />
            </svg>
          </button>
        </div>
      </div>

      {/* ── Messages ── */}
      <div
        className="flex-1 overflow-y-auto px-4 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        style={{ maxHeight: "calc(100vh - 220px)" }}
      >
        <div className="flex flex-col gap-4 py-4">
          <AnimatePresence initial={false}>
            {messages.map((msg) => (
              <motion.div
                key={msg.id}
                initial={{ opacity: 0, y: 6 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.15 }}
                className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
              >
                {msg.role === "assistant" ? (
                  /* ── Assistant message: plain text, no bubble (like original app) ── */
                  <div className="flex max-w-[90%] items-start gap-2.5">
                    {/* Alia avatar */}
                    <div className="mt-0.5 shrink-0">
                      <AliaFace size={24} expression={msg.content ? "idle" : "thinking"} />
                    </div>
                    <div className="min-w-0 flex-1">
                      {msg.content ? (
                        <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-white/80">
                          {msg.content}
                        </p>
                      ) : (
                        /* Thinking indicator */
                        <div className="flex items-center gap-1 py-1">
                          <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/30" style={{ animationDelay: "0ms" }} />
                          <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/30" style={{ animationDelay: "150ms" }} />
                          <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/30" style={{ animationDelay: "300ms" }} />
                        </div>
                      )}
                    </div>
                  </div>
                ) : (
                  /* ── User message: blur bubble with border (like original app) ── */
                  <div className="max-w-[85%] overflow-hidden rounded-[24px] border border-white/15 bg-white/10 px-4 py-2.5 backdrop-blur-sm">
                    <p className="whitespace-pre-wrap text-[13px] leading-relaxed text-white/90">
                      {msg.content}
                    </p>
                  </div>
                )}
              </motion.div>
            ))}
          </AnimatePresence>
          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* ── Input bar ── */}
      <div className="border-t border-white/8 px-3 py-2.5">
        <div className="flex items-end gap-2">
          {/* Voice */}
          <button
            onClick={toggleListening}
            className={`flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-full transition-all ${
              isListening
                ? "bg-[#ff453a]/25 text-white shadow-[0_0_12px_rgba(255,69,58,0.3)]"
                : "text-white/35 hover:bg-white/8 hover:text-white/55"
            }`}
            aria-label={isListening ? "Stop listening" : "Start voice"}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm-1-9c0-.55.45-1 1-1s1 .45 1 1v6c0 .55-.45 1-1 1s-1-.45-1-1V5z" />
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z" />
            </svg>
          </button>

          {/* Input */}
          <div className="flex min-h-[36px] flex-1 items-center overflow-hidden rounded-[24px] border border-white/12 bg-white/6 transition-colors focus-within:border-white/20 focus-within:bg-white/10">
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Message Alia..."
              rows={1}
              className="max-h-[80px] min-h-[20px] flex-1 resize-none bg-transparent px-4 py-2 text-[13px] leading-snug text-white placeholder-white/25 outline-none"
            />
          </div>

          {/* Send */}
          <button
            onClick={sendMessage}
            disabled={!input.trim() || isStreaming}
            className="flex h-9 w-9 shrink-0 cursor-pointer items-center justify-center rounded-full bg-white/12 text-white/70 transition-all hover:bg-white/20 hover:text-white disabled:cursor-default disabled:opacity-25"
            aria-label="Send"
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <line x1="12" y1="19" x2="12" y2="5" />
              <polyline points="5 12 12 5 19 12" />
            </svg>
          </button>
        </div>
      </div>
    </motion.div>
  );
}
