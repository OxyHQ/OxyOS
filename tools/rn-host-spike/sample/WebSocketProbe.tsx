/**
 * What React Native's WebSocket does on OxyOS (websocket.h, crates/oxyos-net's websocket.rs) against
 * the regression's local server (regress.py, `"http"` in the scenario, whose routes answer WebSocket
 * upgrades too): text, binary as an ArrayBuffer and as a Blob both ways, the protocol chosen, a
 * header sent, a ping heard, messages in order, the close handshake with its code and reason from
 * either side, and the failures that must be visible. scenarios/websocket.json checks every row; an
 * answer still pending after 5 s is shown as `pending`. `held` stays open, for the idle regression
 * (perf.py's `idle`).
 */
import * as React from 'react';
import { Text, View } from 'react-native';

const SERVER = 'ws://127.0.0.1:18646';
const TIMEOUT_MS = 5000;

function settle(run: () => Promise<string>): Promise<string> {
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve('pending'), TIMEOUT_MS);
    run().then(
      (outcome) => {
        clearTimeout(timer);
        resolve(`ok ${outcome}`);
      },
      (error: Error) => {
        clearTimeout(timer);
        resolve(`failed: ${String(error?.message ?? error).split('\n')[0]}`);
      },
    );
  });
}

type Script = (socket: WebSocket, seen: string[], done: (outcome: string) => void) => void;

// One socket to `path`, `script` driving it; resolved with what it says, or rejected with its
// error and close.
function socket(path: string, script: Script, protocols?: string[], headers?: Record<string, string>) {
  return new Promise<string>((resolve, reject) => {
    const seen: string[] = [];
    const ws = new (WebSocket as unknown as new (
      url: string,
      protocols?: string[],
      options?: { headers: Record<string, string> },
    ) => WebSocket)(`${SERVER}${path}`, protocols, headers ? { headers } : undefined);
    let errored = false;
    ws.onerror = () => {
      errored = true;
    };
    ws.onclose = (event) => {
      if (errored) reject(new Error(`error, close ${event.code}`));
      seen.push(`close ${event.code} ${event.reason}`);
    };
    script(ws, seen, resolve);
  });
}

function blobText(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error('unreadable blob'));
    reader.readAsText(blob);
  });
}

const PROBES: Record<string, () => Promise<string>> = {
  text: () =>
    socket('/echo', (ws, _, done) => {
      ws.onopen = () => ws.send('hola ñandú');
      ws.onmessage = (event) => {
        done(String(event.data));
        ws.close();
      };
    }),
  binary: () =>
    socket('/echo', (ws, _, done) => {
      ws.binaryType = 'arraybuffer';
      ws.onopen = () => ws.send(new Uint8Array([0, 1, 254, 255]));
      ws.onmessage = (event) => {
        const bytes = new Uint8Array(event.data as ArrayBuffer);
        done(`${bytes.length} ${Array.from(bytes).join(' ')}`);
        ws.close();
      };
    }),
  blob: () =>
    socket('/echo', (ws, _, done) => {
      ws.binaryType = 'blob';
      ws.onopen = () => ws.send(new Blob(['a blob, both ways']));
      ws.onmessage = (event) => {
        const blob = event.data as Blob;
        blobText(blob).then((text) => {
          done(`${blob.size} ${text}`);
          ws.close();
        });
      };
    }),
  protocol: () =>
    socket(
      '/echo',
      (ws, _, done) => {
        ws.onopen = () => {
          done(ws.protocol ?? 'none');
          ws.close();
        };
      },
      ['chat', 'superchat'],
    ),
  header: () =>
    socket(
      '/header',
      (ws, _, done) => {
        ws.onmessage = (event) => {
          done(String(event.data));
          ws.close();
        };
      },
      undefined,
      { 'X-Probe': 'oxyos' },
    ),
  ping: () =>
    socket('/echo', (ws, _, done) => {
      ws.onopen = () => (ws as unknown as { ping(): void }).ping();
      ws.onmessage = (event) => {
        done(String(event.data));
        ws.close();
      };
    }),
  order: () =>
    socket('/count', (ws, _, done) => {
      const got: string[] = [];
      ws.onmessage = (event) => {
        got.push(String(event.data));
        if (got.length === 50) {
          const inOrder = got.every((text, index) => text === String(index));
          done(`${got.length} ${inOrder ? 'in order' : got.join(',')}`);
          ws.close();
        }
      };
    }),
  close: () =>
    socket('/echo', (ws, seen, done) => {
      ws.onopen = () => ws.close(4000, 'done here');
      const closed = ws.onclose!;
      ws.onclose = (event) => {
        closed.call(ws, event);
        done(`${seen.join(' ')} ${ws.readyState}`);
      };
    }),
  'server close': () =>
    socket('/close', (ws, seen, done) => {
      const closed = ws.onclose!;
      ws.onclose = (event) => {
        closed.call(ws, event);
        done(seen.join(' '));
      };
    }),
  // Closed before the server answered its handshake: an error, then a close of 1006, as on the web.
  'close opening': () =>
    socket('/slow', (ws) => {
      ws.close();
    }),
  // Nothing listens on the port after the server's (regress.py holds it bound, not listening).
  refused: () =>
    new Promise<string>((resolve, reject) => {
      const ws = new WebSocket('ws://127.0.0.1:18647/');
      let errored = false;
      ws.onerror = () => {
        errored = true;
      };
      ws.onclose = (event) =>
        errored ? reject(new Error(`error, close ${event.code}`)) : resolve(`closed ${event.code}`);
    }),
  'not a url': async () => {
    try {
      new WebSocket('ftp://127.0.0.1/');
      return 'opened';
    } catch (error) {
      throw new Error(String((error as Error).message).includes('not a WebSocket URL') ? 'thrown' : String(error));
    }
  },
  // Open and left so, for the idle regression.
  held: () =>
    socket('/echo', (ws, _, done) => {
      ws.onopen = () => done('open');
    }),
};

export default function WebSocketProbe() {
  const [outcomes, setOutcomes] = React.useState<Record<string, string>>({});
  React.useEffect(() => {
    for (const [name, run] of Object.entries(PROBES)) {
      settle(run).then((outcome) => setOutcomes((all) => ({ ...all, [name]: outcome })));
    }
  }, []);
  return (
    <View style={{ flex: 1, padding: 16, gap: 4, backgroundColor: '#fff' }}>
      {Object.keys(PROBES).map((name) => (
        <Text key={name} testID={`probe-${name}`}>
          {`ws ${name}: ${outcomes[name] ?? 'running'}`}
        </Text>
      ))}
    </View>
  );
}
