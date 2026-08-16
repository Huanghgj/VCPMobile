import {
  createRouter,
  createWebHashHistory,
  type RouteRecordRaw,
} from "vue-router";
import ChatView from "../../features/chat/ChatView.vue";

const routes: RouteRecordRaw[] = [
  { path: "/", redirect: "/chat" },
  { path: "/chat", name: "chat", component: ChatView },
  {
    path: "/watch",
    name: "watch",
    component: () => import("../../features/watch/WatchView.vue"),
  },
  {
    path: "/assistant",
    name: "assistant",
    component: () => import("../../features/assistant/AssistantView.vue"),
  },
];

if (import.meta.env.DEV || import.meta.env.VITE_RENDERER_PROBE === "1") {
  routes.push({
    path: "/renderer-v2-probe",
    name: "renderer-v2-probe",
    component: () => import("../../features/chat/AndroidRendererProbeView.vue"),
  });
}

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});
