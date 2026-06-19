#!/usr/bin/env bash
# StoryForge 清空数据（保留连接/预设配置，清角色卡/Campaign/会话）
# 用法：bash reset-data.sh
#
# 清掉：角色卡(cards/characters)、Campaign、会话、实例、知识、任务、摘要、向量、active_campaign
# 保留：LLM 连接(connections)、预设(profiles)、Agent Profile(agent_profile_configs)

set -e

ROOT="$(cd "$(dirname "$0")" && pwd)"
DATA="$ROOT/target/debug/data"

if [ ! -d "$DATA" ]; then
  echo "[reset] 数据目录不存在: $DATA"
  echo "[reset] 先跑一次 dev.sh 让应用创建数据目录"
  exit 1
fi

echo "[reset] 清空前确认（保留连接/预设）："
echo "  角色卡: $(node -e "console.log(require('$DATA/cards.json').length)" 2>/dev/null || echo 0) 张"
echo "  会话:   $(ls "$DATA/conversations/" 2>/dev/null | wc -l) 个"
echo "  Campaign: $(node -e "console.log(require('$DATA/campaigns.json').length)" 2>/dev/null || echo 0) 个"
echo ""

# 停掉运行中的 storyforge（避免写回覆盖）
taskkill //F //IM storyforge.exe 2>/dev/null || true

cd "$DATA"

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

echo "[reset] 清空完成。保留："
echo "  连接:   $(node -e "console.log(require('$DATA/connections.json').connections?.length||0)" 2>/dev/null || echo 0) 个"
echo "  预设:   $(node -e "console.log(require('$DATA/profiles.json').length||0)" 2>/dev/null || echo 0) 个"
echo ""
echo "[reset] 现在可以 bash dev.sh 从头试"
