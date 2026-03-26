/** Sound effect system — lazily initialized to avoid autoplay policy issues */

import signInAudio from "../assets/signIn.mp3";
import sosumiAudio from "../assets/Sosumi.wav";

let audioCtx: AudioContext | undefined;
let signInSound: HTMLAudioElement | undefined;
let errorSound: HTMLAudioElement | undefined;

function getAudioCtx() {
  if (!audioCtx) audioCtx = new AudioContext();
  return audioCtx;
}

function playTone(frequency: number, duration: number, type: OscillatorType = "sine", volume = 0.08) {
  const ctx = getAudioCtx();
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = type;
  osc.frequency.value = frequency;
  gain.gain.value = volume;
  gain.gain.exponentialRampToValueAtTime(0.001, ctx.currentTime + duration);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start();
  osc.stop(ctx.currentTime + duration);
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
  if (!signInSound) signInSound = new Audio(signInAudio);
  signInSound.currentTime = 0;
  signInSound.play().catch(() => {});
}

export function playError() {
  if (!errorSound) errorSound = new Audio(sosumiAudio);
  errorSound.currentTime = 0;
  errorSound.play().catch(() => {});
}

export function playScreenshot() {
  playTone(1200, 0.08, "sine", 0.05);
  setTimeout(() => playTone(1600, 0.06, "sine", 0.04), 60);
}
