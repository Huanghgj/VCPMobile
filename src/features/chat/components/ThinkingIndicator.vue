<template>
  <div class="flex items-center gap-2 py-1 px-2.5">
    <div class="thinking-dots flex gap-1">
      <span></span>
      <span></span>
      <span></span>
    </div>
    <span class="text-xs italic opacity-70">正在思考...</span>
  </div>
</template>

<style scoped>
.thinking-dots span {
  width: 4px;
  height: 4px;
  border-radius: 50%;
  background-color: currentColor;
  opacity: 0.3;
}

.thinking-dots span:nth-child(2) {
  opacity: 0.6;
}

.thinking-dots span:nth-child(3) {
  opacity: 0.9;
}
</style>
