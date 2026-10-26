<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue';

interface BoudoirSlide {
  title: string;
  url: string;
  fallback?: string;
}

const SLIDES: BoudoirSlide[] = [
  { title: '魅魔妈妈·紫晶闺房36D半露', url: '/wallpaper/nova_velvet_1.png', fallback: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/90ebd2a8-6c20-4eb6-a858-393c639217b7.png' },
  { title: '丝绒床回眸·妈妈与朵朵趴姿贴贴', url: '/wallpaper/nova_velvet_2.png', fallback: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/3c76f79d-c5c6-4e1e-bbdd-cb53139b4816.png' },
  { title: '主卧解裙·36D雪乳闷脸乳交', url: '/wallpaper/nova_velvet_3.png', fallback: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/5e08c1ab-c81a-406f-858b-1fbe336182e9.png' },
  { title: '黑金盘龙旗袍·女上位拽链骑乘', url: '/wallpaper/nova_velvet_4.png', fallback: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/f6fcbfed-a5ab-4368-9e97-ff3e510ce468.png' },
  { title: '崩铁姬子妈妈·念台词破防内射', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/be3db05a-778f-4732-ab92-d3118634451c.png' },
  { title: '大小卡提希娅&芙露德莉斯·白丝圣女', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/5026ecfa-2318-4de3-a7b8-d6bf12512c20.png' },
  { title: '第一视角·赤足红甲踩脸训犬POV', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/530be0d9-7ce4-4e63-9260-6d7c7c146f90.png' },
  { title: '单手扣腕·Onee-Shota女上位吞没', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/35638823-20e8-41bc-97eb-0af9c1d9ca4e.png' },
  { title: '单手扣腕·女上位贴耳静音娇喘', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/59b360f6-23c1-40a2-8508-b9ac0f3ea54a.png' },
  { title: 'VIP化妆间·36D雪乳与连裤袜蜜臀双视角', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/ad769679-6c3a-402f-90e2-9bd2ba18d573.png' },
  { title: '客厅沙发·“旺逼招工”纸牌微张特写', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/734b1680-d35d-4393-b929-2c59616b86bc.png' },
  { title: '客厅偷感·蓝色Switch手柄抵穴福利', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/abd7a8f7-5759-41c7-9e46-1241d61fdff6.png' },
  { title: '母女同侍·36D夹棒与朵朵小嘴含头', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/b8d57a34-9fa1-4025-981f-2154b019f09b.png' },
  { title: '后入朵朵·白虎雏穴小肚皮顶起凸痕', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/e144f7b2-2c49-4dff-a60d-441f897bed3c.png' },
  { title: '朵朵骑乘·内射灌满储精小鼓包', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/50bd04c0-f9c2-478b-b917-75231f882142.png' },
  { title: '次卧晨检·朵朵粉嫩闭合一线天POV', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/07eaeabe-ae1d-482e-88ed-f51183e2a3ed.png' },
  { title: '蹲姿负压深喉·咽喉口爆吞精POV', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/372e6a43-cc50-493f-ba33-677acc1e2a9e.png' },
  { title: '扫除口交后·张嘴伸舌吞空认证', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/3bb1c4c3-4591-44f0-a411-1fd801abe27c.png' },
  { title: '闺蜜林薇·试衣间蕾丝内衣双美合拍', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/ed032069-3eab-4411-b972-4531ad9f91da.png' },
  { title: '少妇苏晴·普拉提馆36E乳沟微汗同框', url: 'http://108.165.23.62:6005/pw=huanglei%40./images/novelaigen/d685d165-d157-4cc4-8dee-c9ab852c14ae.png' },
];

const currentIndex = ref(0);
const isPlaying = ref(true);
const isExpanded = ref(false);
const isPureView = ref(false);
let timer: number | null = null;

const currentSlide = computed(() => SLIDES[currentIndex.value] || SLIDES[0]);

const nextSlide = () => {
  currentIndex.value = (currentIndex.value + 1) % SLIDES.length;
};

const prevSlide = () => {
  currentIndex.value = (currentIndex.value - 1 + SLIDES.length) % SLIDES.length;
};

const togglePlay = () => {
  isPlaying.value = !isPlaying.value;
};

const togglePureView = () => {
  isPureView.value = !isPureView.value;
  document.documentElement.classList.toggle('nova-pure-lick-mode', isPureView.value);
};

const startTimer = () => {
  if (timer) clearInterval(timer);
  if (isPlaying.value) {
    timer = window.setInterval(() => {
      nextSlide();
    }, 10000);
  }
};

watch(isPlaying, () => {
  startTimer();
});

onMounted(() => {
  startTimer();
});

onUnmounted(() => {
  if (timer) clearInterval(timer);
  document.documentElement.classList.remove('nova-pure-lick-mode');
});
</script>

<template>
  <div class="nova-boudoir-gallery-root pointer-events-none fixed inset-0 z-0 overflow-hidden">
    <!-- 1. 动态相册呼吸轮播图层 -->
    <Transition name="nova-slide-fade">
      <div
        :key="currentSlide.url"
        class="nova-boudoir-slide absolute inset-0 bg-cover bg-center bg-no-repeat"
        :style="{ backgroundImage: `url('${currentSlide.url}')` }"
      ></div>
    </Transition>

    <!-- 2. 丝绒暗角与微醺柔光纱罩（纯享舔屏模式下自动褪去） -->
    <div
      class="nova-boudoir-veil absolute inset-0 transition-opacity duration-500"
      :class="isPureView ? 'opacity-10' : 'opacity-100'"
    ></div>

    <!-- 3. 右上角悬浮黑丝玫瑰金画廊控制胶囊 -->
    <div
      class="nova-boudoir-hud pointer-events-auto fixed z-[85] flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs text-pink-100 transition-all duration-300"
      style="top: calc(var(--vcp-safe-top, 24px) + 8px); right: 12px;"
    >
      <button
        type="button"
        class="flex items-center gap-1 font-bold tracking-wide text-pink-300 active:scale-95"
        @click="isExpanded = !isExpanded"
      >
        <span class="nova-heart-pulse inline-block">💗</span>
        <span>{{ currentIndex + 1 }}/{{ SLIDES.length }}</span>
      </button>

      <template v-if="isExpanded">
        <span class="max-w-[130px] truncate text-[11px] text-pink-100/90">{{ currentSlide.title }}</span>
        <button type="button" class="nova-hud-btn" @click="prevSlide" title="上一张">◀</button>
        <button type="button" class="nova-hud-btn" @click="togglePlay" title="播放/暂停">{{ isPlaying ? '⏸' : '▶' }}</button>
        <button type="button" class="nova-hud-btn" @click="nextSlide" title="下一张">▶</button>
      </template>

      <button
        type="button"
        class="nova-hud-btn ml-0.5 rounded-full px-1.5 py-0.5 text-[10px] font-bold"
        :class="isPureView ? 'bg-pink-500 text-white shadow-lg shadow-pink-500/50' : 'bg-pink-500/25 text-pink-200'"
        @click="togglePureView"
      >
        {{ isPureView ? '退出舔屏' : '🔥舔屏' }}
      </button>
    </div>
  </div>
</template>

<style scoped>
.nova-boudoir-slide {
  animation: novaKenBurns 11s ease-in-out infinite alternate;
  will-change: transform, opacity;
}

@keyframes novaKenBurns {
  0% { transform: scale(1.0); }
  100% { transform: scale(1.06); }
}

.nova-boudoir-veil {
  background:
    radial-gradient(circle at 50% 35%, rgba(28, 10, 42, 0.28) 0%, rgba(15, 5, 24, 0.68) 100%),
    linear-gradient(180deg, rgba(16, 6, 26, 0.55) 0%, rgba(22, 8, 34, 0.32) 45%, rgba(14, 5, 22, 0.72) 100%);
}

.nova-boudoir-hud {
  background: rgba(26, 10, 40, 0.72);
  backdrop-filter: blur(16px) saturate(190%);
  -webkit-backdrop-filter: blur(16px) saturate(190%);
  border: 1px solid rgba(255, 105, 165, 0.45);
  box-shadow: 0 4px 18px rgba(255, 64, 140, 0.28), inset 0 1px 4px rgba(255, 182, 218, 0.25);
}

.nova-hud-btn {
  padding: 2px 5px;
  border-radius: 9999px;
  transition: all 0.2s ease;
}
.nova-hud-btn:active {
  transform: scale(0.9);
  background: rgba(255, 92, 156, 0.35);
}

.nova-heart-pulse {
  animation: novaHeartBeat 1.6s ease-in-out infinite;
}
@keyframes novaHeartBeat {
  0%, 100% { transform: scale(1); filter: drop-shadow(0 0 4px rgba(255, 92, 156, 0.8)); }
  50% { transform: scale(1.22); filter: drop-shadow(0 0 10px rgba(255, 64, 140, 1)); }
}

.nova-slide-fade-enter-active,
.nova-slide-fade-leave-active {
  transition: opacity 1.1s ease-in-out;
}
.nova-slide-fade-enter-from,
.nova-slide-fade-leave-to {
  opacity: 0;
}
</style>
