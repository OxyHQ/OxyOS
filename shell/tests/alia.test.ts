import { afterEach, describe, expect, mock, test } from "bun:test";
import { oxyClient } from "@oxyhq/core";
import { streamChat } from "../src/lib/alia";

const originalFetch = globalThis.fetch;

async function collect(stream: AsyncGenerator<string>): Promise<string> {
  let output = "";
  for await (const chunk of stream) output += chunk;
  return output;
}

afterEach(() => {
  oxyClient.clearTokens();
  globalThis.fetch = originalFetch;
});

describe("OxyOS Alia transport", () => {
  test("fails closed before the network when no Oxy session exists", async () => {
    oxyClient.clearTokens();
    const fetchMock = mock(() => Promise.reject(new Error("fetch must not run")));
    globalThis.fetch = fetchMock as typeof fetch;

    await expect(collect(streamChat([{ role: "user", content: "hello" }]))).rejects.toThrow(
      "Sign in with your Oxy account to use Alia.",
    );
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("calls the product runtime with the live Oxy bearer and exact profile id", async () => {
    oxyClient.setTokens("test-oxy-session-token");
    const fetchMock = mock(async () =>
      new Response('data: {"choices":[{"delta":{"content":"Hi"}}]}\n\ndata: [DONE]\n\n', {
        status: 200,
        headers: { "Content-Type": "text/event-stream" },
      }),
    );
    globalThis.fetch = fetchMock as typeof fetch;

    await expect(collect(streamChat([{ role: "user", content: "hello" }]))).resolves.toBe("Hi");
    expect(fetchMock).toHaveBeenCalledTimes(1);

    const [url, init] = fetchMock.mock.calls[0] ?? [];
    expect(url).toBe("https://api.alia.onl/alia/chat");
    expect(init?.headers).toEqual({
      "Content-Type": "application/json",
      Authorization: "Bearer test-oxy-session-token",
    });

    const body = JSON.parse(String(init?.body)) as { model: string; stream: boolean };
    expect(body.model).toBe("profile:lite");
    expect(body.stream).toBe(true);
  });

  test("contains no browser-bundled Alia key fallback or retired route/model", async () => {
    const source = await Bun.file(new URL("../src/lib/alia.ts", import.meta.url)).text();
    expect(source).not.toContain("VITE_ALIA_API_KEY");
    expect(source).not.toContain("/v1/chat/completions");
    expect(source).not.toContain('"alia-lite"');
  });
});
