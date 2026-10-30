/**
 * What React Native's networking does on OxyOS (network.h): fetch, XMLHttpRequest and their
 * bodies against the regression's local server (regress.py, `"http"` in the scenario), plus the
 * failures that must be visible. scenarios/network.json checks every row; an answer still pending
 * after 5 s is shown as `pending`. A multipart form's file is one in the scenario's home, which the
 * file service opens (files.h); one outside it is refused.
 */
import * as React from 'react';
import { Text, View } from 'react-native';
import { oxyosSurfaces } from './oxyos/platform-surface';

const SERVER = 'http://127.0.0.1:18642';
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

// One XMLHttpRequest, resolved with what its handlers saw.
function xhr(configure: (request: XMLHttpRequest) => void, body?: string): Promise<string> {
  return new Promise((resolve, reject) => {
    const request = new XMLHttpRequest();
    configure(request);
    request.onload = () => resolve(`${request.status} ${request.responseText}`);
    request.ontimeout = () => resolve('timed out');
    request.onerror = () => reject(new Error('xhr error'));
    request.send(body);
  });
}

const PROBES: Record<string, () => Promise<string>> = {
  'fetch json': async () => JSON.stringify(await (await fetch(`${SERVER}/json`)).json()),
  'fetch text': async () => (await fetch(`${SERVER}/text`)).text(),
  'fetch status': async () => String((await fetch(`${SERVER}/missing`)).status),
  'fetch header': async () => (await fetch(`${SERVER}/json`)).headers.get('x-oxyos') ?? 'none',
  'fetch post': async () =>
    (await fetch(`${SERVER}/echo`, {
      method: 'POST',
      headers: { 'Content-Type': 'text/plain' },
      body: 'hola',
    })).text(),
  'fetch redirect': async () => {
    const response = await fetch(`${SERVER}/redirect`);
    return `${response.url.replace(SERVER, '')} ${JSON.stringify(await response.json())}`;
  },
  'fetch bytes': async () => {
    const bytes = new Uint8Array(await (await fetch(`${SERVER}/bytes`)).arrayBuffer());
    return `${bytes.length} ${bytes[0]} ${bytes[255]}`;
  },
  'fetch form': async () => {
    const form = new FormData();
    form.append('name', 'OxyOS');
    return (await fetch(`${SERVER}/echo`, { method: 'POST', body: form })).text();
  },
  'fetch form file': async () => {
    const places = await oxyosSurfaces.files!.places();
    const home = places.find(({ kind }) => kind === 'home')!.uri;
    const form = new FormData();
    form.append('file', { uri: `${home}probe.txt`, name: 'probe.txt', type: 'text/plain' } as never);
    return (await fetch(`${SERVER}/echo`, { method: 'POST', body: form })).text();
  },
  'fetch form outside': async () => {
    const form = new FormData();
    form.append('file', { uri: 'file:///etc/hostname', name: 'hostname', type: 'text/plain' } as never);
    return (await fetch(`${SERVER}/echo`, { method: 'POST', body: form })).text();
  },
  'fetch abort': async () => {
    const controller = new AbortController();
    setTimeout(() => controller.abort(), 100);
    await fetch(`${SERVER}/slow`, { signal: controller.signal });
    return 'not aborted';
  },
  // Nothing listens on the port after the server's (regress.py holds it bound, not listening).
  'fetch refused': async () => (await fetch('http://127.0.0.1:18643/')).text(),
  'xhr header': () =>
    xhr((request) => {
      request.open('GET', `${SERVER}/header`);
      request.setRequestHeader('X-Probe', 'oxyos');
    }),
  // A text body as it arrives: a character split between two reads arrives whole.
  'xhr progress': () =>
    new Promise((resolve, reject) => {
      const request = new XMLHttpRequest();
      const seen: string[] = [];
      request.open('GET', `${SERVER}/split`);
      request.onprogress = () => seen.push(request.responseText);
      request.onload = () => resolve(`${seen.filter(Boolean).join('|')} ${request.responseText}`);
      request.onerror = () => reject(new Error('xhr error'));
      request.send();
    }),
  'xhr timeout': () =>
    xhr((request) => {
      request.open('GET', `${SERVER}/slow`);
      request.timeout = 200;
    }),
};

export default function NetworkProbe() {
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
          {`${name}: ${outcomes[name] ?? 'running'}`}
        </Text>
      ))}
    </View>
  );
}
