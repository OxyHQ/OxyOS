const ALIA_API = "https://api.alia.onl/v1/chat/completions";
const ALIA_MODEL = "alia-lite";

interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

/**
 * Stream chat completions from the Alia API.
 * Yields content deltas as they arrive.
 */
export async function* streamChat(
  messages: ChatMessage[],
  apiKey?: string,
): AsyncGenerator<string> {
  const key = apiKey || import.meta.env.VITE_ALIA_API_KEY;

  const headers: Record<string, string> = {
    "Content-Type": "application/json",
  };
  if (key) {
    headers["Authorization"] = `Bearer ${key}`;
  }

  const res = await fetch(ALIA_API, {
    method: "POST",
    headers,
    body: JSON.stringify({
      model: ALIA_MODEL,
      messages: [
        {
          role: "system",
          content:
            "You are Alia, the AI assistant built into OxyOS. Be concise, helpful, and friendly. Keep responses short unless the user asks for detail.",
        },
        ...messages,
      ],
      stream: true,
    }),
  });

  if (!res.ok) {
    throw new Error(`Alia API error: ${res.status}`);
  }

  const reader = res.body?.getReader();
  if (!reader) throw new Error("No response body");

  const decoder = new TextDecoder();
  let buffer = "";

  while (true) {
    const { done, value } = await reader.read();
    if (done) break;

    buffer += decoder.decode(value, { stream: true });
    const lines = buffer.split("\n");
    buffer = lines.pop() || "";

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed.startsWith("data: ")) continue;
      const data = trimmed.slice(6);
      if (data === "[DONE]") return;

      try {
        const parsed = JSON.parse(data);
        const delta = parsed.choices?.[0]?.delta?.content;
        if (delta) yield delta;
      } catch {
        // Skip malformed chunks
      }
    }
  }
}
