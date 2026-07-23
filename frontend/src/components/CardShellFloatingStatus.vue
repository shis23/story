<script setup>
import { nextTick, ref } from 'vue'
import { Dialog, DialogPanel, DialogTitle } from '@headlessui/vue'

const expanded = ref(false)
const orbRef = ref(null)
const closeRef = ref(null)

async function openStatus() {
  expanded.value = true
  await nextTick()
  closeRef.value?.focus()
}

async function closeStatus() {
  expanded.value = false
  await nextTick()
  orbRef.value?.focus()
}
</script>

<template>
  <div class="fixed bottom-5 right-5 z-[var(--z-overlay)] sm:bottom-7 sm:right-7">
    <button
      ref="orbRef"
      type="button"
      aria-label="打开当前状态"
      :aria-expanded="expanded"
      class="group grid h-12 w-12 place-items-center rounded-full border border-accent-border bg-surface text-accent shadow-float transition hover:scale-105 hover:bg-accent hover:text-white focus:outline-none focus:ring-2 focus:ring-accent/40"
      @click="openStatus"
    >
      <span class="relative h-5 w-5 rounded-full border-2 border-current" aria-hidden="true">
        <span class="absolute inset-[4px] rounded-full bg-current"></span>
      </span>
      <span class="sr-only">当前状态</span>
    </button>

    <Dialog
      :open="expanded"
      :initial-focus="closeRef"
      class="relative z-[var(--z-toast)]"
      @close="closeStatus"
    >
      <div class="fixed inset-0 bg-ink/35" aria-hidden="true" />
      <div class="fixed inset-3 sm:inset-6">
        <DialogPanel class="flex h-full flex-col overflow-hidden rounded-2xl border border-line bg-bg shadow-float">
          <header class="flex shrink-0 items-center gap-3 border-b border-line bg-surface px-5 py-3">
            <span class="h-2 w-2 rounded-full bg-accent"></span>
            <DialogTitle class="flex-1 text-sm font-semibold text-ink">当前状态</DialogTitle>
            <button
              ref="closeRef"
              type="button"
              aria-label="关闭当前状态"
              class="min-h-8 rounded-md px-3 text-xs text-ink-soft hover:bg-surface-2 hover:text-ink"
              @click="closeStatus"
            >关闭</button>
          </header>
          <div class="min-h-0 flex-1 overflow-hidden p-3 sm:p-5">
            <div class="h-full">
              <slot />
            </div>
          </div>
        </DialogPanel>
      </div>
    </Dialog>
  </div>
</template>
