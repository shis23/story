# StoryForge 鏋舵瀯璇存槑

> 鏇存柊鏃ユ湡锛?026-07-14
> 鏈枃鎻忚堪褰撳墠 `main` 鐨勪唬鐮佽竟鐣屻€傚巻鍙叉灦鏋勫揩鐓т綅浜?`docs/archive/`銆?
## 鏋舵瀯鍘熷垯

1. Campaign 鏄暱鏈熸晠浜嬬姸鎬佺殑涓荤嚎鐪熺浉婧愩€?2. 姝ｆ枃銆丄ttempt銆丆ampaign revision 鍜?MutationBatch 蹇呴』閫氳繃 Turn 鐢熷懡鍛ㄦ湡涓€鑷存彁浜ゃ€?3. app/domain 灞備笉渚濊禆 Tauri锛汿auri 鏄粍鍚堟牴鍜屾湰鍦板瓨鍌ㄩ€傞厤灞傘€?4. LLM 鍙互浣跨敤鍚嶅瓧浜ゆ祦锛岃惤鐩樺拰鎺堟潈蹇呴』浣跨敤绋冲畾 ID銆?5. JSON 鏄粯璁ゅ悗绔紱SQLite 鍙兘閫氳繃鏄惧紡 opt-in銆乫ail-closed cutover 鍜屽彲閫嗗鍑哄惎鐢ㄣ€?6. Harness 搴斿鐢ㄧ敓浜у簲鐢ㄦ湇鍔★紝涓嶉暱鏈熺淮鎶ょ浜屽鐘舵€佹満鎴栤€滆繎浼肩敓浜р€濆疄鐜般€?7. 鎵€鏈夌湡瀹炴ā鍨嬨€丟UI銆佽澶囧拰鍙戝竷澹版槑蹇呴』涓庤瘉鎹瓑绾х粦瀹氥€?
## Workspace 杈圭晫

```text
frontend
  -> Tauri commands / events

crates/tauri-app
  -> composition root
  -> command DTO / local app services
  -> JSON stores / SQLite runtime selector

crates/app-pipeline
  -> Director / Subagent / Editor orchestration

crates/app-agent
  -> runtime / prompts / tools / quality / postprocess parsing

crates/app-conversation
  -> conversation tree / variants / provenance

crates/app-memory
  -> archived summaries / recall

crates/app-meta
  -> diagnostics / explanations / patches / MVU analysis

crates/domain
  -> Campaign / Turn / Chronicle / NarrativeContract / LLM DTO

crates/infra-*
  -> LLM / SQLite / import / plugin host / vector / regex / util

crates/harness-real-llm
  -> deterministic gates / real-model probes / M5 evidence
```

褰撳墠 workspace 鍏?16 涓?crate銆俙domain` 涓嶄緷璧栧唴閮ㄥ簲鐢ㄥ眰锛沗app-*` 涓嶄緷璧?`tauri-app`锛汿auri 璐熻矗鎶?store 缁勮涓哄簲鐢ㄥ眰浣跨敤鐨勫揩鐓у拰鏈嶅姟銆?
## 鍐欎綔涓?Turn 鐢熷懡鍛ㄦ湡

```text
User intent
  -> Tauri adapter
  -> PipelineOrchestrator
     -> Director emits Plan + ScenePlan
     -> Subagents run with per-instance ToolContext
     -> Editor composes draft
     -> Conversation variant + provenance
  -> QualityGate
     -> optional 1x Editor-only auto-fix
  -> TurnRecord / TurnAttempt draft_hash sync
  -> background Summarizer + PostProcessor
     -> candidate summary / knowledge / variables / tasks
     -> Attempt guarded writeback
  -> user Accept / force Accept
  -> TurnLifecycleService
     -> scope checks
     -> revision CAS
     -> finalize variant
     -> apply MutationBatch
     -> Committed or Degraded
```

鏍稿績涓嶅彉閲忥細

- 鍙湁娲诲姩 Attempt 鍙互鍐欏洖銆?- 鏃?Attempt 鐨勮繜鍒扮粨鏋滀笉鑳借鐩栨柊鑽夌銆?- `draft_hash` 蹇呴』瀵瑰簲鏈€缁堣繑鍥炵粰鐢ㄦ埛鐨勬鏂囷紝鍖呮嫭 auto-fix 绋裤€?- force accept 鐨勭洰鏍囩粓鎬佹槸 `Degraded`锛屾仮澶嶆椂涓嶅緱鍙樺洖 `Committed`銆?- Campaign / conversation scope 涓嶅尮閰嶆椂蹇呴』 fail closed銆?- 鎺ュ彈杩囩▼鐨勫瓨鍌ㄥけ璐ュ繀椤讳紶鎾紝涓嶅厑璁稿搷搴旀垚鍔熻€岀鐩樼姸鎬佹粸鍚庛€?
## Postprocess 杈圭晫

鎴愭枃鍚庣殑鈥滃悗澶勭悊闃舵鈥濆寘鍚袱涓亴璐ｄ笉鍚岀殑 Agent锛?
- Summarizer锛氱敓鎴愭湰杞?Chronicle A / RoundSummary銆?- PostProcessor锛氱敓鎴愮煡璇嗐€佸彉閲忓拰浠诲姟鍊欓€夋洿鏂般€?
褰撳墠 Tauri 鍐欎綔鍛戒护璐熻矗鍚庡彴缂栨帓銆丄ttempt 鍚屾鍜屾寔涔呭寲閫傞厤銆侻5 harness 宸插鐢ㄧ敓浜у啓浣?Pipeline 涓庡叡浜?Accept 鏈嶅姟锛屼絾浠嶄娇鐢ㄦ槑纭爣璁扮殑 synthetic Chronicle fixture锛涘洜姝ゅ畬鏁?Summarizer/PostProcessor/Attempt 鍚庡彴鍐欏洖灏氭湭鎴愪负鍙敱 Tauri 涓?harness 鍏卞悓璋冪敤鐨勫叡浜簲鐢ㄦ湇鍔°€?
涓嬩竴鏋舵瀯鍒囩墖搴旀娊鍑?`ProductionPostprocessService`锛岀粺涓€锛?
- Summarizer / PostProcessor 璋冪敤涓庡彇娑堛€?- MutationBatch normalize 涓?ID/scope 鏍￠獙銆?- quality report銆乨raft hash 鍜?Attempt 鐘舵€佸悓姝ャ€?- Chronicle A 鍙戝竷銆佸悜閲忕储寮曞拰鍘嬬缉璋冨害銆?- 杩熷埌缁撴灉銆侀噸璇曘€佸箓绛夋仮澶嶄笌澶辫触浼犳挱銆?
Tauri command 鍙仛 DTO銆佷簨浠跺拰鍚庡彴浠诲姟閫傞厤锛沨arness 鐩存帴璋冪敤鍏变韩鏈嶅姟锛屼笉鍚姩 GUI銆?
## Memory / Context / Chronicle

- `H_anchor=5`銆乣E=10` 鏄綋鍓嶇敓浜ч粯璁わ紝涓嶅緱绉颁负宸叉爣瀹氬弬鏁般€?- ContextEpoch 鍥哄畾鍚屼竴 epoch 鐨?anchor銆乷verview銆乥and 鍜?revision 瑙嗗浘銆?- Director 鍙娇鐢?`search_chronicle` / `get_chronicle` 鏌ヨ Chronicle A/B/C銆?- ChronicleCompressor job/publication 鍩虹鏀寔 A鈫払鈫扖銆佽繛缁潪閲嶅彔 covers銆乣covered_by`銆乺evision 涓庡箓绛?replay銆?- MemoryArchiver 澶勭悊瀵硅瘽娑堟伅褰掓。锛屼笌 Chronicle A/B/C 鏄笉鍚屾按浣嶅拰鐢ㄩ€斻€?
鏉冨▉瑙勬牸锛歚docs/MEMORY-CONTEXT-COMPILER-SPEC-2026-07-11.md`銆?
## LLM Request Policy

- active connection 鐨?temperature銆乼op_p銆乺easoning銆乪xtra 鍜屾樉寮忚緭鍑轰笂闄愪細娉ㄥ叆 Pipeline/AgentRuntime銆?- 榛樿 `max_tokens=None`锛孫penAI-compatible 璇锋眰浣撶渷鐣ヨ瀛楁銆?- 鍘嗗彶鏈爣璁扮殑 `Some(4096)` 瑙嗕负鏃?UI 榛樿骞跺綊涓€涓?`None`銆?- 鐢ㄦ埛鏄惧紡濉啓姝ｆ暣鏁版椂閫忎紶锛涙ā鍨嬫敮鎸佺殑鏈€澶ц緭鍑哄彧鏄?ceiling 鑳藉姏锛屼笉浠ｈ〃姣忚疆搴旂敓鎴愯闀垮害銆?- 杩炴帴 ping銆丣SON fallback銆丮emoryArchiver 鍜岃瘎浼伴绠楀彲浠ユ湁鍚勮嚜鐨勪笓鐢ㄩ檺鍒躲€?
閲嶈瘯绛栫暐鍜?provider capability 鎺㈡祴浠嶆湭褰㈡垚瀹屾暣缁熶竴鐨?`RequestPolicy` 鏈嶅姟锛涘綋鍓嶄富瑕佽В鍐充簡閲囨牱鍙傛暟鎰忓浘鍜屼富鍐欎綔閫忎紶闂銆?
## 瀛樺偍鍚庣

### JSON锛堥粯璁わ級

JSON stores 浠嶆槸鏈?opt-in 鐢ㄦ埛鐨勯粯璁ゆ潈濞佹暟鎹簮銆俆urn journal銆乺evision銆丮utationBatch 鍜屾仮澶嶉€昏緫鎻愪緵涓撶敤閫昏緫鍘熷瓙鎬э紝浣嗕笉鏄暟鎹簱浜嬪姟銆?
### SQLite锛堟樉寮?opt-in锛?
SQLite 鍚庣褰撳墠鍏峰锛?
- `StorageBackend::{Json, Sqlite}` 涓庤繘绋?pin銆?- cutover 閿併€佷复鏃跺簱銆佸浠姐€佸唴瀹?hash銆乵arker-last 鍙戝竷鍜屽惎鍔ㄦ仮澶嶃€?- Accept UoW銆乺ecovery銆乤ctive-turn barrier銆?- Chronicle publication UoW 鍜屾晠闅滄敞鍏ュ洖婊氥€?- SQLite鈫扟SON staging + atomic publish reverse export銆?
褰撳墠闄愬埗锛氶儴鍒?pre-accept draft銆丄ttempt 涓棿鎬佸拰 postprocess 鍐欏懡浠や粛闇€瀹屾垚鍏ㄨ矾寰勮縼绉汇€傞粯璁ゅ悗绔笉寰楀湪璇ュ伐浣滃畬鎴愬墠鍒囨崲涓?SQLite銆?
## 鎻掍欢涓庡鍏ヨ竟鐣?
- 鎻掍欢 runtime 鏀寔鏄惧紡鏉冮檺銆佽繍琛屾椂鎾ら攢銆乸rompt-hook timeout/cancel/budget銆佸璁℃煡璇?鍒嗛〉/retention 鍜屾樉寮?degraded/unsupported 鍏煎鐭╅樀銆?- FNV-1a audit chain 浠呮槸鏈湴瀹屾暣鎬ч摼锛屼笉鏄瘑鐮佸绛惧悕鎴栧彲淇″ご璇佹槑銆?- ST/涓栫晫涔?Campaign Bundle 瀵煎叆鎵ц fail-closed 寮曠敤鏍￠獙涓庤ˉ鍋垮洖婊氾紱鐪熷疄澶嶆潅鍗′粛闇€鍦ㄥ悎娉?fixture 鐜琛ヨ瘉鎹€?- 鎻掍欢 iframe銆佺湡瀹炵涓夋柟鎵╁睍鍜屽畬鏁?ST 闀垮熬璇箟浠嶉渶 GUI 楠屾敹銆?
## 鍙戝竷杈圭晫

- 鏈湴 workspace銆佸墠绔拰 host-side release runner 宸叉湁鑷姩鍖栬瘉鎹€?- Gitea workflow 宸叉彁浜わ紝浣嗚繙绔?runner 鎵ц灏氭湭楠岃瘉銆?- Windows bundle銆丄ndroid APK銆佺鍚嶃€丟UI 鍜岀湡鏈鸿瘉鎹繀椤诲湪 `docs/RELEASE-CHECKLIST.md` 鍗曠嫭璁板綍銆?- M5 褰撳墠涓?45/100 Partial Evidence锛屼笖 `production_postprocess_complete=false`銆?
## 褰撳墠涓昏鎶€鏈€?
1. 鍏变韩 ProductionPostprocessService銆?2. SQLite pre-accept 鍏ㄧ敓鍛藉懆鏈熻縼绉汇€?3. 瀹屾暣鐢熶骇璺緞 M5 100-Accept銆?4. Gitea runner 涓庡彲绂荤嚎楠岃瘉鐨勭湡瀹炰骇鐗╄瘉鎹€?5. GUI銆丄ndroid 鐪熸満鍜岀涓夋柟鎻掍欢鐜板満鐭╅樀銆?
