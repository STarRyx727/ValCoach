# ValCoach V4 — Map + Semantic Replay + Compact Agent + Personalized Memory

你现在作为本项目的：

- 资深 Rust 工程师
- C# / .NET 集成工程师
- VALORANT Replay 数据工程师
- Agent 系统工程师
- 前端可视化工程师
- 数据库与长期记忆系统工程师
- 测试与调试负责人

继续开发现有项目：

# ValCoach
**Personalized Evidence-Grounded VALORANT Replay Coaching Agent**

不要新建一个完全独立的项目。

第一步必须先检查当前 workspace、已有 Rust 代码、Replay Adapter、数据库、前端、Agent、第三方 Parser checkout 和现有文档，然后在现有架构上增量实现。

---

# 一、ValCoach 的最终目标

ValCoach 不应该只是：

```text
上传 .vrf
→ 解析
→ 输出几十万行 JSON
→ 把整个 JSON 塞给大模型
→ 输出一段泛泛建议
```

正确目标是：

```text
VALORANT .vrf
        ↓
Replay Parser
        ↓
Semantic Replay Compiler
        ↓
Compact Replay
        ↓
Evidence Store
        ↓
Personalized Memory
        ↓
Question Router / Agent Tools
        ↓
按问题选择相关回合与证据
        ↓
必要时回查 Raw Replay
        ↓
LLM
        ↓
带地图、回合、时间和证据的个性化 Coaching
```

用户最终应该可以问：

```text
我 A 点防守哪里有问题？
```

系统能够自动完成：

```text
识别用户
↓
找到防守方回合
↓
筛选发生在 A 区的相关回合
↓
查看双方站位
↓
查看枪口方向
↓
查看射击 / 伤害 / 击杀
↓
查看技能
↓
查看 Spike
↓
比较多个回合
↓
结合历史个性化问题
↓
给出有证据的 Coaching
```

---

# 二、当前 Replay Parser 的状态：不要误判问题

当前生产主 Parser：

```text
michel-giehl/ValorantReplayParser
```

GitHub：

```text
https://github.com/michel-giehl/ValorantReplayParser
```

这是 C# Parser。

ValCoach 核心逻辑仍然必须保持 Rust。

正确边界：

```text
C# ValorantReplayParser
        ↓
versioned export bundle
        ↓
Rust Replay Adapter
        ↓
ValCoach Domain
```

不要让 Rust 业务逻辑依赖 C# Parser 内部类型。

---

# 三、Global 13.05 已经基本解析成功

当前 Global Replay：

```text
branch:
++Ares-Core+release-13.05

map:
/Game/Maps/Bonsai/Bonsai

map display name:
Split

duration:
2,092,106 ms
```

`michel-giehl/ValorantReplayParser` 已经在：

```text
2026-09-02
```

加入 Global 13.05 transform。

关键 commit 至少包括：

```text
b51d67423b7b4952d59051cf91e55efa1c42da05
feat: add 13.05 transforms
```

当前真实解析 Bundle 中已经有：

```text
movement.ndjson
parser_events.ndjson
server_events.ndjson
manifest.json
diagnostics.json
```

规模大约：

```text
movement:
165,000+ rows

parser events:
138,000+ rows

decoded export groups:
125,000+

RPC:
4,700+

shots:
3,200+

server events:
244
```

Parser health：

```text
packet_count:
630,158

malformed_packet_count:
0

partial_error_count:
148
```

因此：

# 当前主要问题不是 13.05 Parser 整体解析错误。

不要优先重写 payload transform。

---

# 四、Server Event 已经包含大量 Ground Truth

当前这场 Global Split：

```text
characterDeath          161
characterUltimateUsed    45
roundStarted             20
spikePlanted             14
spikeDefused              2
spikeExploded              1
switchTeams                1

total                    244
```

这些 Event 是服务器写入 Replay 的直接时间线。

应该被用于构建：

```text
Rounds
Combat
Spike
Ultimate
Side Switch
```

---

# 五、当前 Player 身份已经能解析

当前分析用户：

```text
Subject UUID:
ec3ffefe-e11b-5623-8f56-3c55deef5bc1
```

Replay 原始数据已经可以建立：

```text
Subject UUID
        ↓
BombPlayerState NetGUID = 284
        ↓
SpawnedCharacter = 802
        ↓
/Game/Characters/Hunter/Hunter_PC
```

即：

```text
Character = 802
Agent internal = Hunter
Agent display = Sova
```

当前该玩家 raw 数据约有：

```text
movement:
16,000+

shots:
304

kills:
18

deaths:
17
```

因此：

> DeepSeek 之前说“没有对枪、死亡原因”等，并不是 Replay 没解析出来，而是这些数据没有进入 Semantic IR / replay_context。

---

# 六、当前最核心的问题：Semantic Replay Layer 不完整

当前链路实际类似：

```text
Replay Parser
 ↓
大量 Raw Events
 ↓
Rust Adapter
 ↓
只做 movement summary
 ↓
DeepSeek
```

DeepSeek 最后只看到：

```text
平均速度
最大速度
总路径长度
样本数
整局时间范围
```

这对于：

```text
我 A 点防守哪里做错了？
```

几乎没有价值。

因此本轮最重要任务不是 Prompt Engineering。

而是：

# Build the Semantic Replay Layer.

---

# 七、实现 ReplaySemanticBuilder

建立：

```text
Raw Replay Bundle
        ↓
ReplaySemanticBuilder
        ↓
Semantic Replay
```

至少包括：

```text
PlayerResolver
RoundBuilder
CombatBuilder
SpikeBuilder
AbilityBuilder
MovementBuilder
MapAreaResolver
TimelineBuilder
```

---

# 八、PlayerResolver

将：

```text
Subject
BombPlayerState
PlayerInfo
SpawnedCharacter
Agent
```

JOIN。

输出类似：

```json
{
  "subject": "ec3ffefe-e11b-5623-8f56-3c55deef5bc1",
  "player_state_net_guid": 284,
  "character_net_guid": 802,
  "agent_internal": "Hunter",
  "agent": "Sova"
}
```

必须区分：

```text
Subject UUID
PlayerState NetGUID
Character NetGUID
Agent UUID / Agent Path
```

不要混成一个 ID。

---

# 九、RoundBuilder

输入：

```text
server:
roundStarted
switchTeams
```

以及 Parser RPC：

```text
ClientRoundStart
ClientBuyPhaseEnd
MulticastEndRound
MulticastSetPhase
```

输出：

```rust
Round {
    number,
    start_ms,
    combat_start_ms,
    end_ms,
    side,
    winner,
}
```

UI 中：

```text
内部 round index:
0-based

用户显示：
第 1 回合、第 2 回合……
```

---

# 十、所有时间都必须人类可读

Raw 数据继续保留：

```text
809771 ms
```

但 Semantic 层必须转换为：

```text
第 8 回合 · 00:26.1
```

建议：

```rust
ReplayTime {
    absolute_ms,
    round_no,
    round_elapsed_ms,
}
```

Raw 时间只能作为 debug / Evidence。

默认给用户和 LLM 的时间：

```text
R8 00:26.1
```

不要再默认输出：

```text
809771 ms
```

---

# 十一、CombatBuilder

必须从：

```text
characterDeath
MulticastNotifyKilledEnemy
MulticastNotifyDamage_Point
MulticastNotifyDamage_Base
valorant_shot_received
weapon state
movement
```

JOIN 出统一 Combat Event。

例如：

```rust
CombatEvent {
    round_no,
    time,

    attacker,
    victim,

    damage,
    killed,

    hit_region,
    hit_bone,

    weapon,

    attacker_position,
    victim_position,

    attacker_yaw,
    attacker_pitch,

    evidence,
}
```

当前 raw Replay 已经能够出现：

```text
DamageDealt
DamageKilledTarget
DamagedBone
RegionalDamage
DamagerPlayerState
EventInstigatorPawn
weapon
shot yaw
shot pitch
ammo
attack vectors
```

不要把这些事实丢掉。

---

# 十二、SpikeBuilder

输入：

```text
spikePlanted
spikeDefused
spikeExploded
TimedBomb actor
spike state
```

输出：

```rust
SpikeEvent {
    round,
    time,
    type,
    player,
    position,
    site,
}
```

当前已经能够发现：

```text
server spikePlanted
```

附近存在：

```text
/Game/GameModes/Bomb/TimedBomb
```

actor。

TimedBomb 有世界坐标。

因此可以恢复：

```text
下包时间
下包世界位置
A / B Site
```

---

# 十三、地图必须加入网站

目前网站只有文字。

这严重限制 ValCoach。

前端应该参考：

```text
ValoPlant
```

的交互地图风格。

不要直接复制 ValoPlant 的私有资源。

地图静态数据优先使用：

```text
Valorant-API
```

接口：

```text
https://valorant-api.com/v1/maps
```

其 Maps 数据包含：

```text
displayName
displayIcon
listViewIcon
splash
mapUrl
xMultiplier
yMultiplier
xScalarToAdd
yScalarToAdd
callouts
```

特别重要：

```text
callouts[].regionName
callouts[].superRegionName
callouts[].location
```

可以提供：

```text
A Main
A Site
A Tower
A Screens
Mid
B Main
...
```

---

# 十四、地图资源建议

首次运行或开发期间，将地图缓存成：

```text
assets/maps/
├─ split/
│   ├─ minimap.png
│   └─ metadata.json
├─ ascent/
├─ bind/
├─ haven/
├─ sunset/
├─ lotus/
├─ pearl/
├─ fracture/
├─ breeze/
├─ icebox/
├─ abyss/
├─ corrode/
└─ ...
```

不要每次页面刷新都重新请求远程地图。

---

# 十五、世界坐标必须转换成地图坐标

Raw Replay：

```text
x
y
z
```

不能直接显示给用户。

需要：

```text
World Coordinates
        ↓
Valorant map transform
        ↓
Normalized Minimap Coordinates
        ↓
Pixel Coordinates
```

社区常见 VALORANT 转换方式为：

```text
map_x = world_y * xMultiplier + xScalarToAdd
map_y = world_x * yMultiplier + yScalarToAdd
```

注意世界 X / Y 与地图轴存在交换。

实现后必须使用真实 Replay 点验证：

```text
玩家点是否落在地图正确位置
Spike 是否落在正确 Site
```

不要只相信公式。

---

# 十六、MapAreaResolver

V1：

```text
world coordinate
↓
map coordinate
↓
nearest callout
↓
A Main / A Tower / ...
```

V2 应升级为：

```text
区域 polygon
```

因为 nearest callout 在上下层 / 隔墙情况下可能错误。

建议接口：

```rust
trait MapResolver {
    fn world_to_map(&self, pos: Vec3) -> MapPoint;

    fn area_at(&self, pos: Vec3) -> Option<MapArea>;
}
```

Semantic position：

```rust
MapPosition {
    world,
    normalized,
    region,
    super_region,
}
```

---

# 十七、用户和 LLM 默认不再看到 Raw Coordinate

不要显示：

```text
x = 7337.1
y = -7022.7
z = 0
```

默认显示：

```text
A Site
```

或者：

```text
A Site · 靠近 A Main
```

Movement 路线：

```text
A Tower → A Site → Screens
```

Raw coordinate 只作为 debug / Evidence 保留。

---

# 十八、Movement 必须增强

当前 movement 最终不应该只是：

```text
time
position
velocity
```

应该至少：

```rust
MovementSample {
    round,
    time,

    player,

    position,
    velocity,

    yaw,
    pitch,

    alive,

    area,
}
```

特别检查：

```text
yaw
pitch
```

是否在：

```text
C# Parser
→ NDJSON
→ Rust Adapter
```

某一层丢失。

对于 Coaching：

```text
站在哪里
```

和：

```text
枪口看哪里
```

必须同时有。

这决定：

```text
架枪
漏角
转身
pre-aim
crosshair placement
背身死亡
```

等判断。

---

# 十九、网站应升级为 2D Replay Viewer

参考 ValoPlant。

页面建议：

```text
┌──────────────────────────────┐
│            Split             │
│                              │
│      Enemy ▲                 │
│                   ● You      │
│                              │
│        A Main      A Site    │
│                              │
└──────────────────────────────┘
```

下面：

```text
第 8 回合

00:20 ─── ● ───── ● ───── ● ── 00:35
          射击      受伤      死亡
```

右侧：

```text
AI Coaching

你第一次接触后继续停留 A Site……
```

地图至少支持：

```text
玩家位置
敌人位置
Spike
死亡位置
移动轨迹
事件节点
当前时间
播放 / 暂停
拖动时间轴
Round 切换
```

---

# 二十、Token 消耗问题：绝不能继续把 Raw Replay 塞给 LLM

当前单局数据：

```text
数十 MB
几十万 JSON records
```

如果全部进入 Prompt：

```text
几十万 token
```

这是不可接受的。

需要：

# Two-Level Replay Compaction

---

# 二十一、第一层 Compact 不使用 LLM

第一层必须是 Rust deterministic compactor。

原因：

> 如果为了省最终 Agent Token，先把几十万 Token 发给 Compact Agent，本身也很贵。

因此：

```text
Raw Replay
    ↓
Rust Semantic Compiler
    ↓
Deterministic Compact Replay
```

这一层：

```text
0 LLM tokens
```

---

# 二十二、Movement Compaction

例如：

```text
16634 movement points
```

不要保存成：

```text
624 ms
632 ms
640 ms
...
```

压缩成：

```json
{
  "round": 8,

  "segments": [
    {
      "start": "00:04.2",
      "end": "00:10.8",
      "from": "A Tower",
      "to": "A Site"
    },

    {
      "start": "00:10.8",
      "end": "00:24.6",
      "area": "A Site",
      "state": "holding"
    }
  ]
}
```

算法可以结合：

```text
area change
speed threshold
direction change
combat event
ability event
spike event
```

切分 segment。

---

# 二十三、Shot Compaction

不要永远给 LLM 304 条独立 shot。

合并：

```text
single shot
burst
spray
```

形成：

```json
{
  "time": "00:26.1",
  "area": "A Site",
  "weapon": "Vandal",
  "shots": 4,
  "hits": 1,
  "result": "headshot_kill"
}
```

---

# 二十四、Compact Replay 必须按 Round 存储

不要只有巨大：

```text
compact.json
```

建议：

```text
compact/
├─ match.json
├─ players.json
├─ round_01.json
├─ round_02.json
├─ ...
└─ round_20.json
```

每个 round 大约只保存：

```text
side
result
route
key positions
combat
abilities
spike
important timing
```

---

# 二十五、第二层才使用 Compact Agent

第二层输入：

```text
Semantic Round
```

而不是：

```text
Raw NDJSON
```

可以使用便宜模型。

作用：

```text
事实
↓
高层战术标签
```

例如：

```json
{
  "round": 8,

  "compact_analysis": {
    "role": "A anchor",

    "first_contact": "00:24.8",

    "retreated_after_contact": false,

    "death_context":
      "died holding A Site against pressure from A Main",

    "trade_available": false
  }
}
```

这一层输出必须：

```text
结构化
短
可验证
```

不要写几千字自然语言总结。

---

# 二十六、最终 Question Agent 也不加载全部 Compact

用户问：

```text
我 A 点防守有什么问题？
```

Question Router 应先过滤：

```text
side = Defense

area ∈
A Site
A Main
A Tower
A Screens
A Ramps
```

找：

```text
相关 Round
```

例如：

```text
R2
R8
R12
R17
```

只加载这些 compact round。

预计最终问题上下文：

```text
几千到一两万 token
```

而不是：

```text
几十万 token
```

---

# 二十七、Compact 不够时才回查 Raw Evidence

这是非常重要的设计。

不要把 Raw Replay 完全删除。

Raw Replay 是：

```text
Evidence Store
```

Agent 可以调用：

```text
inspect_raw_evidence(
    round,
    start_time,
    end_time,
    players,
    event_types
)
```

例如：

```text
Round 8
00:20 → 00:30

types:
movement
shot
damage
```

只返回：

```text
那 10 秒相关数据
```

而不是整个 Match。

流程：

```text
Compact Evidence
      ↓
足够？
 ├─ YES → 回答
 └─ NO
      ↓
 RawEvidenceTool
      ↓
 小时间窗口
      ↓
 回答
```

---

# 二十八、Replay 数据最好进入本地 SQLite / Parquet

不要每次问问题重新：

```text
读取 86 MB NDJSON
```

Replay ingest 后：

```text
Raw Bundle
↓
SQLite / Parquet
↓
Semantic Replay
↓
Compact Replay
```

需要 index：

```text
match_id
round
time_ms
player
area
event_type
```

这样 RawEvidenceTool 可以快速查询。

---

# 二十九、实现个性化本地长期记忆

不能只保存：

```text
聊天记录
```

应该建立：

# Personalized Coaching Memory

本地 SQLite 即可。

至少包含：

```text
player_profiles
issues
issue_occurrences
coach_sessions
```

---

# 三十、Issue Model

例如：

```text
issue_id:
split_defense_a_overpeek_after_contact

category:
positioning

title:
A 点第一次接触后过度停留

description:
受到 A Main 压力后没有及时退到二线角度

map:
Split

side:
Defense

area:
A

severity:
0.74

confidence:
0.91

status:
active

occurrences:
5
```

---

# 三十一、IssueOccurrence

每次问题出现必须有真实 Evidence：

```text
Match 1
Round 8
00:26.1

Match 3
Round 4
00:31.7

Match 5
Round 11
00:18.3
```

数据库：

```rust
IssueOccurrence {
    issue_id,
    match_id,
    round_no,
    timestamp,
    severity,
    evidence_refs,
}
```

---

# 三十二、Issue 状态

至少：

```text
active
improving
resolved
recurring
```

例如：

```text
最近 6 局出现 4 次
→ active

最近频率下降
→ improving

连续 N 局不再出现
→ resolved

之后重新出现
→ recurring
```

不要让 LLM 自己每次重新从所有历史 Replay 判断趋势。

趋势应该先由 deterministic statistics 计算。

---

# 三十三、个性化 Memory 也可以省 Token

用户问：

```text
我最近最大的问题是什么？
```

不需要加载过去 20 局 Replay。

只给：

```json
{
  "issues": [
    {
      "title": "A 点第一次接触后过度停留",
      "occurrences": 5,
      "trend": "improving",
      "recent_examples": [...]
    }
  ]
}
```

需要证据时：

```text
get_issue_evidence(issue_id)
```

再查。

---

# 三十四、Agent Tools

至少实现：

```text
get_match_summary

get_players

get_rounds

find_rounds_by_area

get_round_timeline

get_player_route

get_combat_events

get_ability_events

get_spike_events

get_area_occupancy

get_nearby_players

get_personal_issues

get_issue_evidence

inspect_raw_evidence
```

---

# 三十五、Agent 回答问题时的正确流程

例如：

```text
用户：
我 A 点防守有什么问题？
```

Agent：

```text
1. resolve current player

2. get defense rounds

3. find rounds involving A

4. load compact round facts

5. load relevant personal issues

6. compare several rounds

7. if evidence insufficient:
   inspect_raw_evidence

8. answer
```

---

# 三十六、DeepSeek 最终应该看到的数据

错误：

```json
{
  "average_speed": 307.71,
  "path_distance": 888427.93,
  "sample_count": 16762
}
```

正确：

```json
{
  "scope": {
    "map": "Split",
    "side": "Defense",
    "site": "A"
  },

  "player": {
    "agent": "Sova"
  },

  "rounds": [
    {
      "round": 8,

      "route": [
        "A Tower",
        "A Site"
      ],

      "events": [
        {
          "time": "00:24.8",
          "area": "A Site",
          "type": "first_contact"
        },

        {
          "time": "00:26.1",
          "area": "A Site",
          "type": "death",
          "killer_area": "A Main",
          "weapon": "Vandal"
        }
      ]
    }
  ],

  "history": [
    {
      "issue":
        "A 点第一次接触后过度停留",

      "occurrences": 5,

      "trend": "improving"
    }
  ]
}
```

---

# 三十七、网站 UI 最终结构建议

页面：

```text
┌───────────────────────────────┐
│ Match / Round Selector        │
├──────────────────────┬────────┤
│                      │        │
│      Map Replay      │ Coach  │
│                      │        │
│  player/enemy/spike  │ AI     │
│  trajectories        │ answer │
│                      │        │
├──────────────────────┴────────┤
│ Round Timeline                │
│ shot / damage / kill / spike  │
└───────────────────────────────┘
```

额外 Tab：

```text
Overview
Rounds
Map Replay
Personal Issues
Chat
```

---

# 三十八、Personal Issues 页面

显示：

```text
当前主要问题
历史出现次数
严重度
趋势
最近证据
地图/区域
建议
```

例如：

```text
A 点首次接触后过度停留

出现：
5 次

趋势：
改善中

最近：
Split R8
Split R11
Split R4
```

点击可以跳到：

```text
对应 Match
对应 Round
对应地图时间点
```

---

# 三十九、不要把这些任务拆成互不相干的 Patch

地图、Compact、Human-readable 时间位置、Memory 是同一条数据链。

统一设计：

```text
Raw Replay
   ↓
Semantic Compiler
   ↓
Humanized Timeline
   ↓
Map Semantics
   ↓
Compact Replay
   ↓
Evidence Store
   ↓
Personal Memory
   ↓
Question Agent
```

不要：

```text
前端自己转换时间
Agent 自己猜区域
数据库再另外计算 Round
Compact Agent 再重新解析 Raw JSON
```

所有语义都应该有一个 authoritative source。

---

# 四十、建议的 Rust Domain Types

至少：

```rust
ReplayMetadata

Player

Round

ReplayTime

MapPosition

MapArea

MovementSample

MovementSegment

ShotBurst

CombatEvent

AbilityEvent

SpikeEvent

TimelineEvent

EvidenceRef

ReplayCapabilities

PersonalIssue

IssueOccurrence
```

---

# 四十一、EvidenceRef

所有高层结论必须能追溯：

```rust
EvidenceRef {
    match_id,
    round_no,
    absolute_ms,
    player_id,
    evidence_type,
    source_event,
}
```

前端可以根据 Evidence：

```text
点击建议
↓
跳到 Round 8 · 00:26.1
↓
地图显示对应位置
```

---

# 四十二、Token / Cost 统计

系统必须统计：

```text
raw replay size
semantic event count
compact size
question context token count
raw fallback token count
LLM input tokens
LLM output tokens
cost
```

UI 最好显示：

```text
本次分析：
8,421 input tokens
1,283 output tokens
```

而不是几十万 Token 默默消耗。

---

# 四十三、Compact Agent 模型可独立配置

配置：

```text
Main Coach Model:
DeepSeek

Compact Model:
可配置较便宜模型
```

但必须允许：

```text
Compact LLM Disabled
```

此时只使用 Rust deterministic compact。

---

# 四十四、缓存 Compact

每个 Match：

```text
Replay SHA256
↓
Semantic version
↓
Compact version
```

只要：

```text
Replay 文件没变
Semantic Builder version 没变
Compact version 没变
```

就不要重新跑 Compact。

---

# 四十五、不要删除 Raw Replay Evidence

Compact 是：

```text
索引 / 摘要
```

Raw 是：

```text
事实来源
```

任何高层结论必须可以按需回查 Raw。

---

# 四十六、当前数据一致性问题需要顺便调查

当前 DeepSeek 曾报告：

```text
movement sample_count:
16762
```

但当前 bundle 同一 player raw movement 约：

```text
16634
```

同时最大速度能对应。

需要检查：

```text
是否重复 ingest
是否用了旧缓存
是否做了 interpolation
是否补点
是否跨 actor
```

新增：

```text
raw movement rows
semantic movement rows
DB movement rows
compact source rows
```

一致性 diagnostics。

如果有 deliberate resample：

```text
明确记录算法
```

---

# 四十七、路径距离也要修正

当前 raw Unreal coordinates 不能直接当 meter。

并且路径距离计算应：

```text
按 Round 分段
按 Life segment 分段
避免 death → respawn 跳跃
过滤异常 teleport
```

UI 应显示：

```text
Raw distance
```

或者在确认 Unreal unit conversion 后再显示：

```text
estimated meters
```

不要未经验证直接叫米。

---

# 四十八、开发优先级

严格按这个顺序：

## Phase 1
PlayerResolver

## Phase 2
RoundBuilder + Humanized Time

## Phase 3
Map API + Coordinate Transform

## Phase 4
MapAreaResolver

## Phase 5
CombatBuilder

## Phase 6
SpikeBuilder

## Phase 7
Movement yaw/pitch + area enrichment

## Phase 8
Deterministic Compact Replay

## Phase 9
Raw Evidence Store + indexed query

## Phase 10
Agent Tools + Question Router

## Phase 11
Compact Agent

## Phase 12
Personalized Issue Memory

## Phase 13
2D Replay UI

如果某些模块现有代码已经部分完成：

> 在现有代码上补齐，不要为了符合顺序而重写。

---

# 四十九、第一个可验证目标

完成前 7 个 Phase 后，系统必须 deterministic 回答：

```text
这个玩家是谁？
```

```text
第 8 回合什么时候开始？
```

```text
第 8 回合 26 秒发生了什么？
```

```text
玩家当时在哪个区域？
```

```text
谁杀了他？
```

```text
对方大概从哪个区域击杀？
```

```text
使用什么武器？
```

```text
Spike 是否下在 A？
```

这些问题不需要 LLM。

---

# 五十、第二个可验证目标

完成 Compact 后：

同一场 Replay：

```text
Raw input:
几十 MB
```

Question Agent 实际 Prompt：

```text
目标 < 20k tokens
```

普通问题尽量：

```text
< 10k tokens
```

只有需要深挖时才调用：

```text
RawEvidenceTool
```

---

# 五十一、最终验收问题

用户：

```text
我 A 点防守有什么问题？
```

最终系统必须：

```text
自动找到相关防守回合
↓
定位 A 区
↓
读取路线和站位
↓
读取双方 combat
↓
读取视角
↓
读取技能
↓
读取 Spike
↓
结合历史个人问题
↓
比较重复模式
↓
输出具体建议
```

回答必须类似：

```text
你在 Split A 点防守中有一个重复问题：
第一次接触后继续停留前点。

R8 00:24.8 你在 A Site 接敌，
之后没有退到 Tower/Screens，
R8 00:26.1 被 A Main 的敌人击杀。

类似情况过去 5 局出现过 4 次，
最近频率有所下降。

建议：
第一接触拿到信息后优先退二线，
避免在没有队友交叉火力的情况下继续停留 Site。
```

并且用户点击：

```text
R8 00:26.1
```

网页地图应该跳到对应时刻。

---

# 五十二、工作原则

必须遵守：

1. 不重新从零写 `.vrf` Parser。
2. 不因为 Parser 有 partial error 就假设所有数据无效。
3. 不把几十 MB Raw JSON 发给 LLM。
4. 不让 LLM 猜 Round。
5. 不让 LLM 猜地图区域。
6. 不让 LLM 自己解析坐标。
7. 不让 Compact Agent重新读取全部 Raw Replay。
8. 不删除 Raw Evidence。
9. 所有高层结论必须有 EvidenceRef。
10. 所有时间默认转换为 `Round + mm:ss`。
11. 所有坐标默认转换为地图区域。
12. Personalized Memory 必须本地持久化。
13. 不支持的数据必须标记 unsupported / partial。
14. 不允许 silent fallback。
15. 每个阶段都必须运行测试和真实 Replay smoke test。

---

# 五十三、每个阶段的汇报格式

每完成一个 Phase，输出：

```text
Completed
- ...

Verified
- command:
- exit code:
- raw input count:
- semantic output count:
- regression result:

Files changed
- ...

Token impact
- before:
- after:

UI impact
- ...

Known limitations
- ...

Next
- ...
```

---

# 五十四、现在开始

首先执行：

# STEP 1 — Inspect Current Workspace

找出：

```text
Replay Parser checkout
Rust replay_adapter
domain model
SQLite schema
movement ingest
parser_events ingest
server_events ingest
movement_summary
replay_context builder
Agent tools
frontend map / replay 页面
current chat history storage
```

然后输出一份：

```text
CURRENT_PIPELINE_AUDIT.md
```

明确：

```text
Raw 数据在哪里
Semantic 数据在哪里
哪些数据已经解析但没有使用
哪些字段在哪一层被丢失
目前每次 LLM 请求为什么消耗这么多 token
```

之后不要等待确认，继续：

# STEP 2 — Humanized Time + RoundBuilder

再继续：

# STEP 3 — Valorant Map Data + MapAreaResolver

再继续后续 Phase。

---

# 最终架构原则

ValCoach V4 应正式采用：

```text
Parser
  ↓
Semantic Compiler
  ↓
Humanized Replay
  ↓
Map Semantics
  ↓
Deterministic Compact
  ↓
Optional Compact Agent
  ↓
Evidence Store
  ↓
Personalized Memory
  ↓
Question Router
  ↓
Raw Evidence Fallback
  ↓
LLM Coach
  ↓
Map + Timeline + Evidence-Based Answer
```

一句话总结：

# 不要让大模型读 Replay；让程序先把 Replay 编译成大模型真正需要的战术事实。