import { oxyClient } from "@oxyhq/core";

const alia = oxyClient.createLinkedClient({ baseURL: "https://api.alia.onl" });
const ALIA_PROFILE = "profile:lite";

interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

function readContentDelta(data: string): string | null {
  try {
    const parsed: unknown = JSON.parse(data);
    if (!parsed || typeof parsed !== "object" || !("choices" in parsed)) return null;

    const choices = parsed.choices;
    if (!Array.isArray(choices)) return null;

    const firstChoice: unknown = choices[0];
    if (!firstChoice || typeof firstChoice !== "object" || !("delta" in firstChoice)) return null;

    const delta = firstChoice.delta;
    if (!delta || typeof delta !== "object" || !("content" in delta)) return null;
    return typeof delta.content === "string" ? delta.content : null;
  } catch (error: unknown) {
    if (!(error instanceof SyntaxError)) throw error;
    return null;
  }
}

/**
 * Stream chat completions from the Alia API.
 * Yields content deltas as they arrive.
 */
export async function* streamChat(
  messages: ChatMessage[],
): AsyncGenerator<string> {
  const res = await alia.client.requestAuthenticatedResponse({
    method: "POST",
    url: "/alia/chat",
    headers: {
      "Content-Type": "application/json",
    },
    body: JSON.stringify({
      model: ALIA_PROFILE,
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

      const delta = readContentDelta(data);
      if (delta) yield delta;
    }
  }
}
