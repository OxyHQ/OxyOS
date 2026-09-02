import { oxyClient } from "@oxyhq/core";

const ALIA_API = "https://api.alia.onl/alia/chat";
const ALIA_PROFILE = "profile:lite";

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
): AsyncGenerator<string> {
  const accessToken = oxyClient.getAccessToken();
  if (!accessToken) {
    throw new Error("Sign in with your Oxy account to use Alia.");
  }

  const res = await fetch(ALIA_API, {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      Authorization: `Bearer ${accessToken}`,
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
