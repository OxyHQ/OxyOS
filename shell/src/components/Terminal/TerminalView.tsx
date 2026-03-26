import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { invoke, isNative } from "../../lib/tauri";

interface TerminalViewProps {
  ptyId: string;
}

export default function TerminalView({ ptyId }: TerminalViewProps) {
  const containerRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    const term = new Terminal({
      cursorBlink: true,
      cursorStyle: "bar",
      fontFamily: "'JetBrains Mono', 'Fira Code', 'SF Mono', 'Menlo', monospace",
      fontSize: 13,
      lineHeight: 1.3,
      allowTransparency: true,
      theme: {
        background: "rgba(0, 0, 0, 0.01)",
        foreground: "rgba(255, 255, 255, 0.85)",
        cursor: "#0a84ff",
        cursorAccent: "#000000",
        selectionBackground: "rgba(255, 255, 255, 0.15)",
        selectionForeground: "#ffffff",
        black: "#1a1a2e",
        red: "#ff453a",
        green: "#30d158",
        yellow: "#ffd60a",
        blue: "#0a84ff",
        magenta: "#bf5af2",
        cyan: "#64d2ff",
        white: "rgba(255, 255, 255, 0.85)",
        brightBlack: "#48484a",
        brightRed: "#ff6961",
        brightGreen: "#4cd964",
        brightYellow: "#ffe620",
        brightBlue: "#409cff",
        brightMagenta: "#da8fff",
        brightCyan: "#70d7ff",
        brightWhite: "#ffffff",
      },
    });

    const fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(container);

    requestAnimationFrame(() => {
      fitAddon.fit();
      setupPty(term, ptyId);
    });

    const resizeObserver = new ResizeObserver(() => {
      fitAddon.fit();
      if (isNative()) {
        invoke("pty_resize", { id: ptyId, cols: term.cols, rows: term.rows });
      }
    });
    resizeObserver.observe(container);

    return () => {
      resizeObserver.disconnect();
      if (isNative()) {
        invoke("pty_kill", { id: ptyId });
      }
      term.dispose();
    };
  }, [ptyId]);

  return <div ref={containerRef} className="h-full w-full px-1 pb-1" />;
}

async function setupPty(term: Terminal, ptyId: string) {
  if (isNative()) {
    const { listen } = await import("@tauri-apps/api/event");

    await invoke("pty_spawn", { id: ptyId, cols: term.cols, rows: term.rows });

    const unlistenData = await listen<string>(`pty-data-${ptyId}`, (event) => {
      term.write(event.payload);
    });

    const unlistenExit = await listen(`pty-exit-${ptyId}`, () => {
      term.write("\r\n\x1b[90m[Process exited]\x1b[0m\r\n");
      unlistenData();
      unlistenExit();
    });

    term.onData((data) => {
      invoke("pty_write", { id: ptyId, data });
    });
  } else {
    setupMockShell(term);
  }
}

function setupMockShell(term: Terminal) {
  const hostname = "oxyos";
  const user = "user";
  let currentDir = "~";
  let inputBuffer = "";

  term.write(`\x1b[1;36mOxTerm\x1b[0m \x1b[90m(Demo Mode)\x1b[0m\r\n`);
  term.write(`\x1b[90mType 'help' for available commands.\x1b[0m\r\n\r\n`);
  printPrompt();

  function printPrompt() {
    term.write(`\x1b[1;32m${user}@${hostname}\x1b[0m:\x1b[1;34m${currentDir}\x1b[0m$ `);
  }

  function handleCommand(cmd: string) {
    const parts = cmd.trim().split(/\s+/);
    const command = parts[0] ?? "";
    const args = parts.slice(1);

    switch (command) {
      case "": break;
      case "help":
        term.write("Available commands: help, echo, clear, whoami, hostname,\r\n");
        term.write("  pwd, date, uname, ls, neofetch\r\n");
        break;
      case "echo": term.write(args.join(" ") + "\r\n"); break;
      case "clear": term.clear(); term.write("\x1b[H\x1b[2J"); break;
      case "whoami": term.write(user + "\r\n"); break;
      case "hostname": term.write(hostname + "\r\n"); break;
      case "pwd": term.write((currentDir === "~" ? `/home/${user}` : currentDir) + "\r\n"); break;
      case "date": term.write(new Date().toString() + "\r\n"); break;
      case "uname": term.write("OxyOS 2.1.0 x86_64 GNU/Linux\r\n"); break;
      case "ls":
        term.write("\x1b[1;34mDesktop\x1b[0m  \x1b[1;34mDocuments\x1b[0m  \x1b[1;34mDownloads\x1b[0m  \x1b[1;34mMusic\x1b[0m  \x1b[1;34mPictures\x1b[0m\r\n");
        break;
      case "neofetch":
        term.write(`\x1b[1;36m       ___       \x1b[0m  ${user}@${hostname}\r\n`);
        term.write(`\x1b[1;36m      /   \\      \x1b[0m  \x1b[90m--------------\x1b[0m\r\n`);
        term.write(`\x1b[1;36m     | O O |     \x1b[0m  \x1b[1mOS:\x1b[0m OxyOS 2.1\r\n`);
        term.write(`\x1b[1;36m     |  ^  |     \x1b[0m  \x1b[1mKernel:\x1b[0m 6.6.x\r\n`);
        term.write(`\x1b[1;36m      \\___/      \x1b[0m  \x1b[1mShell:\x1b[0m bash\r\n`);
        term.write(`\x1b[1;36m     /     \\     \x1b[0m  \x1b[1mDE:\x1b[0m OxyShell\r\n`);
        term.write(`\x1b[1;36m    /_______\\    \x1b[0m  \x1b[1mTerminal:\x1b[0m oxterm\r\n`);
        term.write("\r\n");
        break;
      case "cd":
        if (args.length === 0 || args[0] === "~") currentDir = "~";
        else currentDir = args[0] ?? currentDir;
        break;
      case "exit":
        term.write("\x1b[90m[Process exited]\x1b[0m\r\n");
        return;
      default:
        term.write(`${command}: command not found\r\n`);
    }
    printPrompt();
  }

  term.onData((data) => {
    const code = data.charCodeAt(0);
    if (code === 13) { term.write("\r\n"); handleCommand(inputBuffer); inputBuffer = ""; }
    else if (code === 127) { if (inputBuffer.length > 0) { inputBuffer = inputBuffer.slice(0, -1); term.write("\b \b"); } }
    else if (code === 3) { term.write("^C\r\n"); inputBuffer = ""; printPrompt(); }
    else if (code >= 32) { inputBuffer += data; term.write(data); }
  });
}
