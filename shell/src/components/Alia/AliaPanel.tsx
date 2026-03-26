import { useState, useRef, useEffect, useCallback } from "react";
import { motion } from "framer-motion";
import { useAliaStore } from "../../stores/aliaStore";
import { streamChat } from "../../lib/alia";

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

  // Auto-scroll on new messages
  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  // Focus input on open
  useEffect(() => {
    requestAnimationFrame(() => inputRef.current?.focus());
  }, []);

  const sendMessage = useCallback(async () => {
    const text = input.trim();
    if (!text || isStreaming) return;

    setInput("");
    addMessage("user", text);

    // Build chat history for API
    const store = useAliaStore.getState();
    const chatHistory = [...store.messages, { id: "", role: "user" as const, content: text, timestamp: 0 }]
      .filter((m) => m.id !== "greeting")
      .map((m) => ({ role: m.role, content: m.content }));

    // Add empty assistant message for streaming into
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
      className="fixed top-4 right-4 z-50 flex w-[400px] origin-top-right flex-col rounded-[20px] border border-white/20 bg-white/12 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)] backdrop-blur-[60px] backdrop-saturate-[180%]"
      style={{ maxHeight: "calc(100vh - 120px)" }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-4 pt-3.5 pb-2">
        <div className="flex items-center gap-2">
          <div className="flex h-6 w-6 items-center justify-center rounded-full bg-white/10">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="white" fillOpacity="0.8">
              <path d="M12 2L13.5 8.5L20 7L14.5 11L18 17L12 13.5L6 17L9.5 11L4 7L10.5 8.5L12 2Z" />
            </svg>
          </div>
          <h2 className="text-[14px] font-semibold text-white/90">Alia</h2>
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={clearMessages}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/35 transition-colors hover:bg-white/10 hover:text-white/60"
            aria-label="Clear chat"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M3 6h18M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          </button>
          <button
            onClick={close}
            className="flex h-7 w-7 cursor-pointer items-center justify-center rounded-full text-white/35 transition-colors hover:bg-white/10 hover:text-white/60"
            aria-label="Close"
          >
            <svg width="12" height="12" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round">
              <path d="M1 1l8 8M9 1l-8 8" />
            </svg>
          </button>
        </div>
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 pb-2 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden" style={{ maxHeight: "calc(100vh - 240px)" }}>
        <div className="flex flex-col gap-2.5 py-2">
          {messages.map((msg) => (
            <div
              key={msg.id}
              className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
            >
              <div
                className={`max-w-[85%] rounded-2xl px-3.5 py-2.5 text-[13px] leading-relaxed ${
                  msg.role === "user"
                    ? "bg-[#0a84ff]/25 text-white/90"
                    : "border border-white/8 bg-white/6 text-white/80"
                }`}
              >
                {msg.content || (
                  <span className="inline-flex gap-1">
                    <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/40" style={{ animationDelay: "0ms" }} />
                    <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/40" style={{ animationDelay: "150ms" }} />
                    <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-white/40" style={{ animationDelay: "300ms" }} />
                  </span>
                )}
              </div>
            </div>
          ))}
          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* Input bar */}
      <div className="border-t border-white/8 px-3 py-2.5">
        <div className="flex items-end gap-2">
          {/* Voice button */}
          <button
            onClick={toggleListening}
            className={`flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-colors ${
              isListening
                ? "bg-[#ff453a]/30 text-white"
                : "text-white/40 hover:bg-white/10 hover:text-white/60"
            }`}
            aria-label={isListening ? "Stop listening" : "Start listening"}
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
              <path d="M12 14c1.66 0 3-1.34 3-3V5c0-1.66-1.34-3-3-3S9 3.34 9 5v6c0 1.66 1.34 3 3 3zm-1-9c0-.55.45-1 1-1s1 .45 1 1v6c0 .55-.45 1-1 1s-1-.45-1-1V5z" />
              <path d="M17 11c0 2.76-2.24 5-5 5s-5-2.24-5-5H5c0 3.53 2.61 6.43 6 6.92V21h2v-3.08c3.39-.49 6-3.39 6-6.92h-2z" />
            </svg>
          </button>

          {/* Text input */}
          <textarea
            ref={inputRef}
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Message Alia..."
            rows={1}
            className="max-h-[80px] min-h-[32px] flex-1 resize-none rounded-xl bg-white/8 px-3 py-1.5 text-[13px] leading-snug text-white placeholder-white/30 outline-none transition-colors focus:bg-white/12"
          />

          {/* Send button */}
          <button
            onClick={sendMessage}
            disabled={!input.trim() || isStreaming}
            className="flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full bg-[#0a84ff]/40 text-white transition-all hover:bg-[#0a84ff]/60 disabled:cursor-default disabled:opacity-30"
            aria-label="Send"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="currentColor">
              <path d="M2.01 21L23 12 2.01 3 2 10l15 2-15 2z" />
            </svg>
          </button>
        </div>
      </div>
    </motion.div>
  );
}
