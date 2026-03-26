import { useState, useRef, useEffect, useCallback } from "react";
import { motion, AnimatePresence } from "framer-motion";
import { useAliaStore } from "../../stores/aliaStore";
import { streamChat } from "../../lib/alia";
import AliaFace from "./AliaFace";
import type { AliaExpression } from "./AliaFace";
import AliaWelcome from "./AliaWelcome";
import { oxGlassPresets } from "../../lib/styles";
import OxGlass from "../shared/OxGlass";

function buildHistory(messages: { id: string; role: string; content: string }[]) {
  return messages
    .filter((m) => m.id !== "greeting")
    .map((m) => ({ role: m.role as "user" | "assistant", content: m.content }));
}

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

  // Face expression derived from state
  const lastMessage = messages[messages.length - 1];
  const faceExpression: AliaExpression = !isStreaming
    ? "Idle A"
    : lastMessage?.role === "assistant" && lastMessage.content
      ? "Writing E"
      : lastMessage?.role === "assistant"
        ? "Thinking"
        : "Idle A";

  const runStream = useCallback(async (chatHistory: { role: "user" | "assistant"; content: string }[]) => {
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
  }, [addMessage, appendToLastMessage, setStreaming]);

  const sendMessage = useCallback(async () => {
    const text = input.trim();
    if (!text || isStreaming) return;
    setInput("");
    addMessage("user", text);
    const history = buildHistory([...useAliaStore.getState().messages, { id: "", role: "user", content: text }]);
    await runStream(history);
  }, [input, isStreaming, addMessage, runStream]);

  const handleSuggestion = useCallback((text: string) => {
    setInput("");
    addMessage("user", text);
    const history = buildHistory([...useAliaStore.getState().messages, { id: "", role: "user", content: text }]);
    runStream(history);
  }, [addMessage, runStream]);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      sendMessage();
    }
  };

  const hasText = input.trim().length > 0;
  const showWelcome = messages.length <= 1 && messages[0]?.id === "greeting";

  return (
    <motion.div
      initial={{ opacity: 0, y: -8, scale: 0.97 }}
      animate={{ opacity: 1, y: 0, scale: 1 }}
      exit={{ opacity: 0, y: -8, scale: 0.97 }}
      transition={{ duration: 0.22, ease: [0.2, 0, 0, 1] }}
      className="fixed top-4 right-4 z-50 w-[420px] origin-top-right"
      style={{ maxHeight: "calc(100vh - 80px)" }}
    >
    <OxGlass {...oxGlassPresets.floatingPanel} className="flex flex-col overflow-hidden rounded-[20px] border border-white/20 shadow-[0_8px_40px_rgba(0,0,0,0.35),inset_0_0.5px_0_rgba(255,255,255,0.15)]" style={{ maxHeight: "calc(100vh - 80px)" }}>
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3">
        <div className="flex items-center gap-2">
          <AliaFace size={28} expression={faceExpression} />
          <span className="text-[14px] font-semibold text-white/90">Alia</span>
        </div>
        <div className="flex items-center gap-1">
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

      {/* Messages */}
      <div
        className="flex-1 overflow-y-auto px-4 [-ms-overflow-style:none] [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        style={{ maxHeight: "calc(100vh - 220px)" }}
      >
        <div className="mx-auto flex w-full max-w-3xl flex-col gap-2 pb-4" style={showWelcome ? { flex: 1, justifyContent: "center" } : undefined}>
          {showWelcome && <AliaWelcome onSuggestionPress={handleSuggestion} />}
          <AnimatePresence initial={false}>
            {messages.map((msg, i) => {
              const isLast = i === messages.length - 1;
              return (
                <motion.div
                  key={msg.id}
                  initial={{ opacity: 0, y: 8 }}
                  animate={{ opacity: 1, y: 0 }}
                  transition={{ duration: 0.2 }}
                >
                  {msg.role === "user" ? (
                    <div className="flex flex-col items-end">
                      <p className="max-w-[85%] rounded-[24px] border border-white/15 bg-white/8 px-4 py-2 text-[15px] leading-7 text-white/90 backdrop-blur-sm">
                        {msg.content}
                      </p>
                    </div>
                  ) : (
                    <div className="w-full">
                      {msg.content ? (
                        <p className="whitespace-pre-wrap text-[15px] leading-7 text-white/80">
                          {msg.content}
                        </p>
                      ) : isStreaming && isLast ? (
                        <div className="flex gap-1 py-2">
                          <span className="h-2 w-2 animate-bounce rounded-full bg-white/25" style={{ animationDelay: "0ms" }} />
                          <span className="h-2 w-2 animate-bounce rounded-full bg-white/25" style={{ animationDelay: "150ms" }} />
                          <span className="h-2 w-2 animate-bounce rounded-full bg-white/25" style={{ animationDelay: "300ms" }} />
                        </div>
                      ) : null}
                    </div>
                  )}
                </motion.div>
              );
            })}
          </AnimatePresence>
          <div ref={messagesEndRef} />
        </div>
      </div>

      {/* Input */}
      <div className="border-t border-white/8 px-3 py-2.5">
        <div className="flex items-end gap-2">
          <button
            onClick={toggleListening}
            className={`flex h-8 w-8 shrink-0 cursor-pointer items-center justify-center rounded-full transition-all ${
              isListening
                ? "bg-[#ef4444]/20 text-[#ef4444]"
                : "text-white/35 hover:bg-white/8 hover:text-white/55"
            }`}
            aria-label={isListening ? "Stop recording" : "Voice input"}
          >
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <rect x="9" y="1" width="6" height="12" rx="3" />
              <path d="M5 10a7 7 0 0 0 14 0" />
              <line x1="12" y1="17" x2="12" y2="21" />
              <line x1="8" y1="21" x2="16" y2="21" />
            </svg>
          </button>

          <div className="flex min-h-[40px] flex-1 items-center overflow-hidden rounded-[20px] bg-white/6 transition-colors focus-within:bg-white/10">
            <textarea
              ref={inputRef}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={handleKeyDown}
              placeholder="Ask Alia..."
              rows={1}
              className="max-h-[100px] min-h-[20px] flex-1 resize-none bg-transparent px-[14px] py-[10px] text-[15px] leading-snug text-white placeholder-white/30 outline-none"
            />
          </div>

          {isStreaming ? (
            <button
              onClick={() => setStreaming(false)}
              className="flex h-10 w-10 shrink-0 cursor-pointer items-center justify-center rounded-full bg-[#0a84ff] text-white"
              aria-label="Stop"
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="white">
                <rect x="4" y="4" width="16" height="16" rx="2" />
              </svg>
            </button>
          ) : hasText ? (
            <button
              onClick={sendMessage}
              className="flex h-10 w-10 shrink-0 cursor-pointer items-center justify-center rounded-full bg-[#0a84ff] text-white transition-opacity disabled:opacity-40"
              aria-label="Send"
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round">
                <path d="M12 19V5M5 12l7-7 7 7" />
              </svg>
            </button>
          ) : (
            <button
              onClick={toggleListening}
              className="flex h-10 w-10 shrink-0 cursor-pointer items-center justify-center rounded-full bg-[#0a84ff] text-white"
              aria-label="Voice"
            >
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="white" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M11 5L6 9H2v6h4l5 4V5z" />
                <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
                <path d="M19.07 4.93a10 10 0 0 1 0 14.14" />
              </svg>
            </button>
          )}
        </div>
      </div>
    </OxGlass>
    </motion.div>
  );
}
