#!/usr/bin/env bash
# StoryForge 清空数据（保留连接/预设配置，清角色卡/Campaign/会话）
# 用法：bash reset-data.sh
#
# 清掉：角色卡(cards/characters)、Campaign、会话、实例、知识、任务、摘要、向量、active_campaign
# 保留：LLM 连接(connections)、预设(profiles)、Agent Profile(agent_profile_configs)
#
# Gate 7 后默认存储是 SQLite：脚本按 marker/DB 存在性自动分派
#   - SQLite 权威（storyforge.backend.json 或 storyforge.sqlite3 存在）→ 清库 + marker
#   - JSON 回退（无 SQLite 但 legacy JSON 存在）→ 原逻辑清 JSON
#   - 两者都没有 → 报错（先跑一次 dev.sh）

set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DATA="$ROOT/target/debug/data"
SQLITE_DB="$DATA/storyforge.sqlite3"
MARKER="$DATA/storyforge.backend.json"

if [ ! -d "$DATA" ]; then
  echo "[reset] 数据目录不存在: $DATA"
  echo "[reset] 先跑一次 dev.sh 让应用创建数据目录"
  exit 1
fi

if [ -f "$MARKER" ] || [ -f "$SQLITE_DB" ]; then
  MODE="sqlite"
elif [ -f "$DATA/cards.json" ] || [ -f "$DATA/campaigns.json" ]; then
  MODE="json"
else
  echo "[reset] 数据目录为空（无 SQLite 权威、无 legacy JSON）"
  echo "[reset] 先跑一次 dev.sh 让应用初始化数据目录"
  exit 1
fi

echo "[reset] 存储形态: $MODE"
echo "[reset] 清空前确认（保留连接/预设）："
echo "  角色卡: $(node -e "console.log(require('$DATA/cards.json').length)" 2>/dev/null || echo 0) 张"
echo "  会话:   $(ls "$DATA/conversations/" 2>/dev/null | wc -l) 个"
echo "  Campaign: $(node -e "console.log(require('$DATA/campaigns.json').length)" 2>/dev/null || echo 0) 个"
echo ""

# 停掉运行中的 storyforge（避免写回覆盖）
taskkill //F //IM storyforge.exe 2>/dev/null || true

cd "$DATA"

if [ "$MODE" = "sqlite" ]; then
  # SQLite 权威：删除主库（含 WAL/SHM 侧车）与 marker。JSON 数据文件若存在
  # （旧回退残留）一并清掉；sqlite-backups 保留作安全网，提示用户手动处置。
  rm -f "$SQLITE_DB" "$SQLITE_DB-wal" "$SQLITE_DB-shm"
  rm -f "$MARKER"
  rm -f cards.json characters.json vectors.json campaigns.json instances.json \
        knowledge.json tasks.json round_summaries.json active_campaign.json \
        mvu_translations.json compress_jobs.json
  rm -rf conversations/
  mkdir -p conversations/
  echo "[reset] 已清空 SQLite 权威数据（库 + marker）"
  echo "[reset] 注意：sqlite-backups/ 保留（如需彻底清空请手动删除该目录）"
else
  # JSON 回退模式（原逻辑）
  # 角色卡（两套都清）
  echo '[]' > cards.json
  echo '[]' > characters.json
  # 向量（世界书向量）
  echo '{}' > vectors.json
  # 会话
  rm -rf conversations/
  mkdir -p conversations/
  # Campaign 相关
  echo '[]' > campaigns.json
  echo '[]' > instances.json
  echo '[]' > knowledge.json
  echo '[]' > tasks.json
  echo '[]' > round_summaries.json
  # active campaign 置空
  echo '{"campaign_id":""}' > active_campaign.json
fi

echo "[reset] 清空完成。保留："
echo "  连接:   $(node -e "console.log(require('$DATA/connections.json').connections?.length||0)" 2>/dev/null || echo 0) 个"
echo "  预设:   $(node -e "console.log(require('$DATA/profiles.json').length||0)" 2>/dev/null || echo 0) 个"
echo ""
echo "[reset] 现在可以 bash dev.sh 从头试"
