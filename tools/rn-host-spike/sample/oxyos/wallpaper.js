/**
 * The wallpaper the session's shell shows (docs/adr/0048-wallpapers.mdx), from the shell's state:
 * `wallpaper` is what the host turned the session's choice for this output into, { key, uri, fit,
 * color, slow } (`uri` null for a colour; `slow`, the clock brought it), or null where nothing was
 * told (a host outside a session), which shows the catalog's default, light.
 */
import * as React from 'react';

const FIXTURE = {
  key: 'oxyos/light',
  source: require('../../../../config/includes.chroot_after_packages/usr/share/oxyos/wallpapers/oxyos/light.svg'),
  fit: 'fill',
  color: '#000000',
};

/** The shell's `ShellWallpaper` for the state's `wallpaper`: the same object while it draws the
    same, so the wallpaper and what blurs it render again only when it changes. */
export function useShellWallpaper(told) {
  const key = told ? told.key : null;
  const uri = told ? told.uri : null;
  const fit = told ? told.fit : null;
  const color = told ? told.color : null;
  const slow = told ? !!told.slow : false;
  return React.useMemo(
    () =>
      key === null
        ? FIXTURE
        : { key, source: uri ? { uri } : undefined, fit: fit || 'fill', color: color || '#000000', slow },
    [key, uri, fit, color, slow],
  );
}
