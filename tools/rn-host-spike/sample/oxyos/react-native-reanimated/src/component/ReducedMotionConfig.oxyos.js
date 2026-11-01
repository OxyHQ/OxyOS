// Reanimated's `ReducedMotionConfig` on OxyOS: the mode it is given holds while it is mounted
// (../ReducedMotion.oxyos.js); `ReduceMotion.System` follows the session's Reduce Motion as it
// changes, where Reanimated's own takes the setting once.
import { useEffect } from 'react';
import { logger } from 'react-native-reanimated/src/common';
import { hold } from '../ReducedMotion.oxyos';

export function ReducedMotionConfig({ mode }) {
  useEffect(() => {
    if (!__DEV__) {
      return;
    }
    logger.warn(`Reduced motion setting is overwritten with mode '${mode}'.`);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => hold(mode), [mode]);

  return null;
}
