/** Maps app display names to Linux executables (or executable + args). */
export const appExecMap: Record<string, string> = {
  Browser: "firefox-esr",
  Files: "thunar",
  Settings: "__internal:settings",
  Terminal: "oxterm",
  Email: "firefox-esr https://mail.google.com",
  Maps: "firefox-esr https://maps.google.com",
  Calendar: "__internal:calendar",
  Photos: "eog",
  Camera: "__internal:camera",
  Calculator: "gnome-calculator",
  Clock: "__internal:clock",
  Radio: "firefox-esr https://radio.garden",
  Notes: "gnome-text-editor",
  Docs: "firefox-esr https://docs.google.com",
  Messages: "firefox-esr https://messages.google.com",
  Store: "__internal:store",
};
