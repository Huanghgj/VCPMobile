import { createRouter, createWebHashHistory } from 'vue-router';
import ChatView from '../../features/chat/ChatView.vue';
import PerformanceDiagnosticsView from '../../features/diagnostics/PerformanceDiagnosticsView.vue';

const routes = [
  { path: '/', redirect: '/chat' },
  { path: '/chat', name: 'chat', component: ChatView },
  { path: '/diagnostics', name: 'diagnostics', component: PerformanceDiagnosticsView },
];

export const router = createRouter({
  history: createWebHashHistory(),
  routes,
});
