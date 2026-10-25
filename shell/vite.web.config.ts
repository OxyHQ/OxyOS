import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import reactNativeWeb from 'vite-plugin-react-native-web';

export default defineConfig({
  plugins: [reactNativeWeb(), react(), tailwindcss()],
  resolve: {
    alias: {
      '@react-native/assets-registry/registry': 'react-native-web/dist/modules/AssetRegistry',
    },
  },
  build: {
    outDir: 'web-build',
    target: 'esnext',
    rollupOptions: {
      input: 'web.html',
    },
  },
});
