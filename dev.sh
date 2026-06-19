#!/usr/bin/env bash
# StoryForge 一键启动开发环境
# 用法：bash dev.sh
# 停止：Ctrl+C（会同时杀 vite + tauri）
#
# 做的事：
#   1. 后台起 vite dev server（端口 1420）
#   2. 等 vite 就绪
#   3. 前台起 cargo tauri dev（编译 Rust + 弹窗口）
#   4. Ctrl+C 时清理两个进程

set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
FRONTEND="$ROOT/frontend"
TAURI_APP="$ROOT/crates/tauri-app"
VITE_PORT=1420

cleanup() {
  echo ""
  echo "[dev] 停止中..."
  # 杀 vite 后台进程
  if [ -n "$VITE_PID" ] && kill -0 "$VITE_PID" 2>/dev/null; then
    kill "$VITE_PID" 2>/dev/null || true
  fi
  # 杀 storyforge.exe（tauri dev 启动的）
  taskkill //F //IM storyforge.exe 2>/dev/null || true
  echo "[dev] 已停止"
}
trap cleanup EXIT INT TERM

# 0. 先清掉可能残留的旧进程，避免端口冲突
echo "[dev] 清理残留进程..."
taskkill //F //IM storyforge.exe 2>/dev/null || true
# 杀占用 1420 的旧 vite（如果有）
if command -v netstat >/dev/null 2>&1; then
  OLD_VITE=$(netstat -ano 2>/dev/null | grep ":$VITE_PORT " | grep LISTENING | awk '{print $5}' | head -1)
  if [ -n "$OLD_VITE" ]; then
    echo "[dev] 端口 $VITE_PORT 被占用（PID $OLD_VITE），先杀掉"
    taskkill //F //PID "$OLD_VITE" 2>/dev/null || true
  fi
fi

# 1. 后台起 vite
echo "[dev] 启动 vite dev server（端口 $VITE_PORT）..."
cd "$FRONTEND"
npm run dev > /tmp/storyforge-vite.log 2>&1 &
VITE_PID=$!

# 2. 等 vite 就绪（最多 30 秒）
echo "[dev] 等待 vite 就绪..."
for i in $(seq 1 30); do
  if curl -s -o /dev/null -w "%{http_code}" "http://localhost:$VITE_PORT/" 2>/dev/null | grep -q 200; then
    echo "[dev] vite 就绪（${i}s）"
    break
  fi
  if ! kill -0 "$VITE_PID" 2>/dev/null; then
    echo "[dev] vite 启动失败，日志："
    cat /tmp/storyforge-vite.log
    exit 1
  fi
  sleep 1
done

# 3. 前台起 tauri dev（编译 Rust + 弹窗口）
echo "[dev] 启动 cargo tauri dev（首次编译可能 1-2 分钟，增量约 30s）..."
cd "$TAURI_APP"
cargo tauri dev
