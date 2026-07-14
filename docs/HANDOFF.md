# StoryForge 浜ゆ帴璇存槑

> 鏇存柊鏃ユ湡锛?026-07-14
> 褰撳墠鍩虹嚎锛歚main == origin/main == 99b1ea3`
> 鑼冨洿锛氬綋鍓嶄唬鐮佷簨瀹炪€佽瘉鎹瓑绾с€侀獙璇佸叆鍙ｄ笌涓嬩竴浼樺厛绾с€?
## 褰撳墠缁撹

StoryForge 宸茶繘鍏ュ伐绋嬪寲鍙戝竷鍊欓€夐樁娈点€侰ampaign-first 澶?Agent 鍐欎綔銆乀urn/Attempt 涓€鑷存€с€丳hase B 璐ㄩ噺涓庣瀵嗙煡璇嗛棬绂併€丆hronicle M0鈥揗4.2.2銆佹彃浠?瀵煎叆鍏煎纭寲銆乷pt-in SQLite 鍜屽彂甯冭瘉鎹剼鏈潎宸插舰鎴愬彲鎵ц涓荤嚎銆?
褰撳墠涓嶈兘瀹ｇО姝ｅ紡鍙戝竷鎴栧畬鏁?M5 楠屾敹锛屼富瑕佺己鍙ｆ槸锛?
1. M5 Full 鐪熷疄妯″瀷璇佹嵁浠?45/100 Accept銆?2. M5 harness 鐨勫啓浣滃拰 Accept 浣跨敤鐢熶骇鏈嶅姟锛屼絾 Summarizer/PostProcessor/TurnAttempt 鍚庡彴鍐欏洖涓?Chronicle A 浠嶆湭褰㈡垚鍙緵 harness 澶嶇敤鐨勫畬鏁寸敓浜у簲鐢ㄦ湇鍔°€?3. SQLite opt-in 宸叉垚涓?Accept/recovery/barrier 鐨勬潈濞佽矾寰勶紝浣嗛儴鍒?pre-accept draft/postprocess 鍛戒护浠嶉渶缁х画杩佺Щ锛岄伩鍏嶆贩鍚堝悗绔敓鍛藉懆鏈熴€?4. Gitea runner銆佹闈㈢湡瀹?GUI銆丄ndroid 鐪熸満銆佺鍚嶅畨瑁呭寘鍜岀涓夋柟鎻掍欢 iframe 浠嶇己鐜板満璇佹嵁銆?
## 宸茶惤鍦颁富绾?
| 棰嗗煙 | 褰撳墠鐘舵€?|
| --- | --- |
| Campaign 鍐欎綔 | Director 鈫?Subagent 鈫?Editor 涓昏矾寰勫凡鎺ワ紱ScenePlan銆丯arrativeContract銆乤gency 瀛楁杩涘叆鎻愮ず璇嶄笌 Gate |
| Turn 涓€鑷存€?| TurnRecord / TurnAttempt銆乨raft hash銆乺evision CAS銆丮utationBatch銆丄ccept 灞忛殰銆佽繜鍒?postprocess 瀹堝崼銆佸穿婧冩仮澶嶅凡鎺?|
| Phase B / B2 | 绉佸瘑褰掑睘濂戠害銆佹樉寮忔帰閽堛€佹枃鏈獥鍙?attribution銆丒ditor performance redaction銆?脳 Editor auto-fix 宸叉帴 |
| 璁板繂 / Context | ContextEpoch銆乶ear_raw銆丄/B/C 鏌ヨ宸ュ叿銆丆hronicleCompressor job/publication銆乧ache usage 涓?segment 瑙傛祴宸叉帴 |
| SQLite | 榛樿浠嶄负 JSON锛涙樉寮?opt-in cutover銆乵arker銆丄ccept UoW銆乺ecovery銆乥arrier銆佸浠藉拰 reverse export 宸叉帴 |
| 鎻掍欢 | prompt hook銆佹潈闄愭挙閿€銆侀绠?瓒呮椂/鍙栨秷銆佸璁￠摼銆佸吋瀹圭煩闃典笌鏄惧紡 degraded/unsupported 琛屼负宸叉帴 |
| 瀵煎叆/瀵煎嚭 | ST/Campaign Bundle 鍏煎鐭╅樀銆佸師瀛愬け璐ャ€佸紩鐢ㄦ牎楠屻€乫ixture corpus 鍜岃劚鏁忔姤鍛婂凡鎺?|
| 鍙戝竷璇佹嵁 | Windows/Android host runner銆乵anifest/provenance/hash銆丟itea workflow 宸叉彁浜わ紱杩滅 runner 瀹炶窇灏氭湭楠岃瘉 |

## LLM Request Policy

- 涓诲啓浣滈粯璁?`max_tokens=None`锛岃姹備綋鐪佺暐璇ュ瓧娈碉紝鐢?endpoint/model 鍐冲畾榛樿杈撳嚭涓婇檺銆?- 鍘嗗彶鏈爣璁扮殑 `4096` 瑙嗕负鏃?UI 榛樿锛屼笉浼氱獊鐒跺彉鎴愮敓浜х‖涓婇檺銆?- 鐢ㄦ埛鏄惧紡濉啓姝ｆ暣鏁版椂鎵嶅彂閫佷笂闄愶紱`4096`銆乣384000` 绛夊潎浼氭寜鐢ㄦ埛鎰忓浘閫忎紶銆?- M5 鍙敤 `STORYFORGE_EVAL_MAX_TOKENS` 瑕嗙洊璇勪及璇锋眰锛涜鍊肩幇鍦ㄤ綔鐢ㄤ簬瀹為檯 `ChatRequest`锛屼笉鏄繛鎺ュ璞′笂鐨勬棤鏁堝瓧娈点€?- 涓撶敤璇锋眰浠嶅彲鏈夌嫭绔嬩笂闄愶紝渚嬪杩炴帴 ping銆丣SON fallback 鍜?MemoryArchiver锛涗笉寰楁妸瀹冧滑鎻忚堪鎴愪富鍐欎綔闄愬埗銆?
## M5 / Phase B 璇佹嵁

鏉冨▉缁撴灉锛歚docs/workstreams/M5-PHASEB-100TURN-EVIDENCE-RESULT.md`銆?
| 闃舵 | Accept | Calls | Epoch | 缁撹 |
| --- | ---: | ---: | ---: | --- |
| Canary | 3/3 | 30/30 | 1 | Pass |
| Coverage | 12/12 | 106/120 | 1 | Pass |
| Stability | 30/30 | 282/300 | 3 | Pass |
| Full | 45/100 | 415/700 | 5 | Partial Evidence |

璇佹嵁杈圭晫锛?
- 鍐欎綔鍏ュ彛锛歱roduction pipeline銆?- Accept锛氬叡浜?`TurnLifecycleService` / production-faithful commit probe銆?- Chronicle锛氬綋鍓嶄粛鍚?`synthetic_chronicle_fixture`锛宍production_postprocess_complete=false`銆?- 鐜版湁 45/100 璇佹嵁鐩綍淇濈暀鍦?`C:\tmp\endurance-evidence-full`锛屼笉鍦?Git 杩借釜鑼冨洿銆?- 涓嶅緱鎹淇敼鎴栧绉板凡鏍囧畾 `200/4`銆乣H_anchor=5`銆乣E=10`銆?
## 瀛樺偍杈圭晫

榛樿琛屼负淇濇寔 JSON锛岄伩鍏嶅湪鏈畬鎴愬叏鐢熷懡鍛ㄦ湡杩佺Щ鍓嶅己鍒跺垏鎹㈢敤鎴锋暟鎹€?
SQLite opt-in 宸茶鐩栵細

- 绫诲瀷鍖?backend selector 涓庤繘绋?pin銆?- fail-closed cutover銆乵arker銆侀攣銆佸唴瀹?hash 閲嶇畻鍜屽惎鍔ㄦ仮澶嶃€?- Turn Accept / recovery / active-turn barrier 鏉冨▉璺緞銆?- Chronicle publication UoW銆佹晠闅滄敞鍏ュ洖婊氥€佸浠藉拰 SQLite鈫扟SON reverse export銆?
浠嶉渶琛ラ綈锛?
- `append_ai_draft`銆丄ttempt 涓棿鎬併€乤utofix/postprocess 鍐欏洖绛夊畬鏁?pre-accept 鐢熷懡鍛ㄦ湡銆?- Windows/Android 鐪熸鍚敤 SQLite 鍚庣殑鏂囦欢閿併€佺敓鍛藉懆鏈熷拰澶ф暟鎹幇鍦洪獙璇併€?
## 2026-07-14 楠岃瘉璁板綍

- `cargo fmt --all -- --check`锛氶€氳繃銆?- `cargo clippy --workspace --all-targets -- -D warnings`锛氶€氳繃銆?- `cargo test --workspace`锛氶€氳繃锛涚湡瀹炴ā鍨嬨€丱S credential store 绛夌敤渚嬫寜璁捐 ignored銆?- `cargo test -p harness-real-llm`锛氬叏閮ㄧ‘瀹氭€?suite 閫氳繃锛涚湡瀹炴ā鍨嬬敤渚嬫寜璁捐 ignored銆?- `frontend npm.cmd test`锛?11/311 閫氳繃銆?- `frontend npm.cmd run build`锛氶€氳繃锛涗繚鐣欐棦鏈?Vite dynamic/static import warning銆?- 鏈疆鏈柊澧炵湡瀹?浠樿垂妯″瀷璋冪敤銆?
## 楠岃瘉鍏ュ彛

瀹屾暣纭畾鎬у彂甯冮椄闂細

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\verify-release.ps1
```

鐪熷疄妯″瀷 smoke 鍙粠鐜鍙橀噺璇诲彇鍑瘉锛?
```powershell
$env:LLM_BASE_URL='https://your-compatible-endpoint/v1'
$env:LLM_API_KEY='<secret-from-shell>'
$env:LLM_MODEL='<model>'
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\run-real-llm-smoke.ps1 -Suite knowledge
```

M5 endurance 榛樿涓嶅彂閫佽緭鍑轰笂闄愶紱濡傞渶鏄惧紡 ceiling锛?
```powershell
$env:STORYFORGE_EVAL_MAX_TOKENS='384000'
```

## 涓嬩竴浼樺厛绾?
1. 鎶藉嚭鍏变韩 ProductionPostprocessService锛氳 Tauri 涓?harness 澶嶇敤 Summarizer銆丳ostProcessor銆丄ttempt 鍚屾銆丆hronicle A 鍙戝竷鍜岃繜鍒扮粨鏋滃畧鍗€?2. 鍦ㄥ畬鏁寸敓浜?postprocess 璺緞鎺ラ€氬悗杩愯鏂扮殑 M5 100-Accept 璇佹嵁锛涚幇鏈?45/100 浣滀负鏃ц矾寰勫熀绾夸繚鐣欍€?3. 瀹屾垚 SQLite pre-accept draft/postprocess 鍏ㄧ敓鍛藉懆鏈熻縼绉讳笌鏁呴殰娉ㄥ叆銆?4. 閮ㄧ讲骞跺疄璺?Gitea runner锛岄獙璇?workflow銆佷笂浼犲寘銆乻ubject/sidecar 鍜岀绾?re-hash銆?5. 鐢变汉宸ヨˉ妗岄潰 GUI銆丄ndroid 鐪熸満銆佺鍚嶅寘鍜岀湡瀹炵涓夋柟鎻掍欢楠屾敹銆?
## 浜ゆ帴绾︽潫

- 涓嶆妸鐪熷疄 API key 鍐欏叆浠撳簱銆佹枃妗ｃ€佹棩蹇椼€佹埅鍥炬垨璇佹嵁 JSONL銆?- 涓嶆妸妯″瀷鏈€澶ц兘鍔涚獥鍙ｇ瓑鍚屼簬姣忔璇锋眰搴旇缃殑杈撳嚭闀垮害锛沗max_tokens` 鏄?ceiling锛屼笉鏄洰鏍囬暱搴︺€?- 涓嶆妸 deterministic fixture銆佸彲鎵ц鍏ュ彛鎴?synthetic Chronicle 鍐欐垚瀹屾暣鐢熶骇楠屾敹銆?- 涓嶉粯璁ゅ垏鎹?SQLite锛涘繀椤讳繚鐣?fail-closed cutover銆佸浠藉拰 reverse export銆?- `docs/archive/**` 涓庢棫 workstream PLAN/RESULT 鏄巻鍙茶瘉鎹紝闄ら潪淇浜嬪疄閿欒锛屽惁鍒欎笉鍥炲啓鎴愬綋鍓嶇姸鎬併€?
