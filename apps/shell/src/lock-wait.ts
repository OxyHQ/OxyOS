/**
 * How the lock and login screens' password field says a wait after too many wrong passwords
 * (docs/adr/0010-lock-screen.mdx): the session's throttle makes the fifth in a row wait 30 s,
 * doubling up to 15 minutes, and the field counts it down.
 */

/** A wait as said: "30 segundos", "2 minutos" (the visible hint counts it in seconds). */
export function waitSaid(seconds: number): string {
  if (seconds < 60 || seconds % 60 !== 0) return seconds === 1 ? "1 segundo" : `${seconds} segundos`;
  const minutes = seconds / 60;
  return minutes === 1 ? "1 minuto" : `${minutes} minutos`;
}

/**
 * Whether a wait counting down is said again with `left` seconds left: each whole minute, then
 * half a minute and ten seconds before the end. Never each second: VoiceOver says a status text
 * when it appears and not again as it changes in place (a Mac's lock says its wait once), and a
 * count said every second would drown out the typing echo and the person's own reading. These
 * few reminders and the moment the wait ends tell someone who cannot see the count when to try.
 */
export function waitReminder(left: number): boolean {
  return left === 30 || left === 10 || (left > 0 && left % 60 === 0);
}
