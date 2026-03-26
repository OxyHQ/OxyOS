/** Lightweight sound effect system using Web Audio API — no external files needed */

const audioCtx = new AudioContext();

function playTone(frequency: number, duration: number, type: OscillatorType = "sine", volume = 0.08) {
  const osc = audioCtx.createOscillator();
  const gain = audioCtx.createGain();
  osc.type = type;
  osc.frequency.value = frequency;
  gain.gain.value = volume;
  gain.gain.exponentialRampToValueAtTime(0.001, audioCtx.currentTime + duration);
  osc.connect(gain);
  gain.connect(audioCtx.destination);
  osc.start();
  osc.stop(audioCtx.currentTime + duration);
}

export function playClick() {
  playTone(800, 0.05, "sine", 0.04);
}

export function playNotification() {
  playTone(880, 0.1, "sine", 0.06);
  setTimeout(() => playTone(1100, 0.15, "sine", 0.06), 120);
}

export function playLock() {
  playTone(440, 0.15, "sine", 0.05);
  setTimeout(() => playTone(330, 0.2, "sine", 0.04), 100);
}

export function playUnlock() {
  playTone(330, 0.1, "sine", 0.05);
  setTimeout(() => playTone(520, 0.15, "sine", 0.06), 80);
}

export function playScreenshot() {
  playTone(1200, 0.08, "sine", 0.05);
  setTimeout(() => playTone(1600, 0.06, "sine", 0.04), 60);
}
