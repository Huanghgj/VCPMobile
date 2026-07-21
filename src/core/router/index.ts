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
    path: "/assistant",
    name: "assistant",
    component: () => import("../../features/assistant/AssistantView.vue"),
  },
];

if (import.meta.env.DEV) {
  routes.push({
    path: "/renderer-v2-probe",
    name: "renderer-v2-probe",
    component: () => import("../../features/chat/RendererV2ProbeView.vue"),
  });
}

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});
