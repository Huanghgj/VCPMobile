import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import UnoCSS from "unocss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [
    vue(),
    UnoCSS(),
  ],

  build: {
    chunkSizeWarningLimit: 900,
    rollupOptions: {
      output: {
        manualChunks(id) {
          if (!id.includes('node_modules')) return;

          if (
            id.includes('/node_modules/vue/') ||
            id.includes('/node_modules/vue-router/') ||
            id.includes('/node_modules/pinia/') ||
            id.includes('/node_modules/@vue/') ||
            id.includes('/node_modules/@vueuse/')
          ) {
            return 'vendor-vue';
          }

          if (
            id.includes('/node_modules/@tauri-apps/api/') ||
            id.includes('/node_modules/@tauri-apps/plugin-opener/')
          ) {
            return 'vendor-tauri';
          }

          if (id.includes('/node_modules/highlight.js')) {
            return 'vendor-highlight';
          }

          if (
            id.includes('/node_modules/marked') ||
            id.includes('/node_modules/marked-highlight') ||
            id.includes('/node_modules/dompurify') ||
            id.includes('/node_modules/morphdom')
          ) {
            return 'vendor-markdown';
          }

          if (id.includes('/node_modules/katex')) {
            return 'vendor-katex';
          }

          if (id.includes('/node_modules/pdfjs-dist')) {
            return 'vendor-pdf';
          }

          if (id.includes('/node_modules/mammoth')) {
            return 'vendor-docx';
          }

          if (id.includes('/node_modules/vue-cropper')) {
            return 'vendor-cropper';
          }

          if (id.includes('/node_modules/sortablejs')) {
            return 'vendor-sortable';
          }

          if (
            id.includes('/node_modules/lucide-vue-next') ||
            id.includes('/node_modules/date-fns')
          ) {
            return 'vendor-ui';
          }

          if (
            id.includes('/node_modules/@braintree') ||
            id.includes('/node_modules/@xmldom') ||
            id.includes('/node_modules/jszip') ||
            id.includes('/node_modules/pako') ||
            id.includes('/node_modules/saxes') ||
            id.includes('/node_modules/sax')
          ) {
            return 'vendor-docx';
          }
        },
      },
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    host: host || '0.0.0.0',
    strictPort: true,
    hmr: host
      ? {
          protocol: "ws",
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
