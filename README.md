# innen（因縁）

> **An opinionated, high-performance, deterministic realization of the LLM Wiki paradigm.**
>
> 依據 Andrej Karpathy 的 LLM Wiki 思想，以 Rust 打造具備因果追溯（Dependent Origination）、時序圖譜、雙引擎衍生索引與次線性 Token 檢索的現代知識系統。

---

## 1. 核心定位：LLM Wiki 的一種實作方法

2026 年 4 月，Andrej Karpathy 在其發布的 [LLM Wiki 指南](https://gist.github.com/karpathy/442a6bf555914893e9891c11519de94f) 與後續論述中指出：

> *「在 LLM agent 時代，分享具體的程式碼或應用程式已經不再必要；你只需要分享核心概念（idea file），每個人的代理人就會依據自己的具體需求客製化實作。」*

Karpathy 定義了 **LLM Wiki** 的本質：
- **持續複利的實體（Persistent, Compounding Artifact）**：知識不是每次對話臨時推導，而是被編譯成持久的網絡；交叉引用已經在那裡，矛盾已被標記，綜合反映了所讀過的一切。
- **三層架構**：不可變的原始材料（`01-raw/`） $\to$ LLM 維護的結構化知識（`02-wiki/`） $\to$ 產出、綱要與索引（`03-output/`、`04-index/`）。
- **三種核心操作**：Ingest（編譯萃取）、Query（檢索並將新結論寫回）、Lint（定期健康檢查與消歧）。
- **四大優勢**：**明確（Explicit）**、**自主持有（Yours）**、**檔案優先（File over app）**、**自帶模型（BYOAI）**。

### `innen` 的回答

**`innen`（因縁）是 Karpathy「LLM Wiki」範式的一種高確定性、系統級工程實作。**

在個人百篇筆記的小規模下，純文字 Markdown 搭配簡單目錄尚可運作；然而當知識庫擴展至數百次對話、跨專案實驗矩陣、法律與科研證據鏈時，純文字 Wiki 會迅速遭遇三重瓶頸：
1. **Context Window 與成本膨脹**：LLM 每次檢索動輒需傾印數千字的總帳或 Wiki 全文（2,500 ~ 5,000 tokens），稀釋注意力且大幅拉高延遲。
2. **時序遺失與歷史覆寫**：單純覆寫 Markdown 無法回答「某項決策是在何時被哪次實驗推翻？原始脈絡為何？」。
3. **隱性幻覺與懸空參照**：缺少強型別約束與參照完整性（Reference Integrity），破碎連結與幽靈專案會悄然滋生。

`innen` 取名自佛教哲學的**「因緣」（Dependent Origination，互為條件、因果相續）**，透過**因果事件日誌（Event Journal）**、**雙引擎衍生索引（Redb + Tantivy）**與**混合圖譜檢索（BM25 + 3-Hop BFS）**，在保持「檔案優先、Markdown 本位」的前提下，為 LLM Agent 賦予毫秒級響應與 90%~95% 的極致 Token 減量。

---

## 2. 架構核心：三層責任與引擎解耦

```text
┌─────────────────────────────────────────────────────────────────────────┐
│                           The LLM Agent                                 │
│          (Claude / Gemini / Codex / Antigravity / Open-Source)          │
└───────────────────▲─────────────────────────────────┬───────────────────┘
                    │ query / project (<35ms)         │ ingest / append
                    │ (tokens 節省 90%~95%)           │ (append-only)
┌───────────────────┴─────────────────────────────────▼───────────────────┐
│                              innen CLI                                  │
│                 (Single 12MB Rust Static Binary)                        │
├────────────────────────────────┬────────────────────────────────────────┤
│  Derived Indexes (Rebuild 30ms)│        Source of Truth (Append-Only)   │
│  ┌──────────────────────────┐  │  ┌──────────────────────────────────┐  │
│  │   Tantivy CJK FTS        │  │  │   .innen/journal.jsonl           │  │
│  │   (BM25 + Jieba 分詞)    │  │  │   (Canonical JSON, SHA256 ID,    │  │
│  ├──────────────────────────┤  │  │    Bi-temporal: observed/valid)  │  │
│  │   redb (Embedded KV)     │  │  └──────────────────────────────────┘  │
│  │   (Transactions, Adjacency│ │  ┌──────────────────────────────────┐  │
│  └──────────────────────────┘  │  │   01-raw/ & 02-wiki/ Markdown    │  │
│                                │  └──────────────────────────────────┘  │
└────────────────────────────────┴────────────────────────────────────────┘
```

1. **真實來源唯讀不可變（Event-Sourced Journal）**：
   - 所有的節點（Node）與關係（Edge）異動皆寫入 `.innen/journal.jsonl`。
   - 撤銷關係採用 `edge.retract`，歷史永久保留，永不破壞時間序列。
   - 具備雙時間軸：`observed_utc`（記錄觀察時間）與 `valid_from` / `valid_to`（實體有效區間）。
2. **雙衍生索引（Dual Derived Index）**：
   - **Tantivy CJK FTS**：內嵌 Jieba-rs 中文分詞字典，支援繁體中文 BM25 相關性評分。
   - **Redb**：純 Rust 實作之嵌入式 ACID 交易資料庫，毫秒級處理節點與鄰接表。
   - **索引完全可拋棄**：執行 `innen rebuild` 僅需 **30 毫秒** 即可由 journal 重新生成全部索引。
3. **混合子圖檢索（Hybrid Subgraph Retrieval）**：
   - 檢索機制：BM25 快速篩選 Top 候選種子 $\to$ 沿著關聯邊展開 1~3 步 BFS 圖譜擴散 $\to$ 依距離與長度歸一化權重排序。
   - 相比傳統 Flat RAG，只輸出高度相關的高密度結構化知識行，消弭無關雜訊。

---

## 3. 實測評測：效能與 Token 減量

針對包含 635 筆日誌事件、395 條關聯邊、85 篇 Wiki 之真實知識庫進行全量對比評測：

### A. 效能與資源佔用（Apple Silicon）

| 操作項目 (Command) | Python `km` 引擎 | `innen` (Rust) 引擎 | **加速比 (Speedup)** | 記憶體常駐 (Peak RSS) |
|---|---|---|---|---|
| **CLI 啟動 (`--help`)** | 642.8 ms | **17.3 ms** | **37.1x** | **2.9 MB** (節省 88%) |
| **健康檢查 (`lint`)** | 633.1 ms | **33.4 ms** | **18.9x** | **5.5 MB** (節省 78%) |
| **狀態摘要 (`status`)** | 295.7 ms | **29.4 ms** | **10.0x** | **6.6 MB** (節省 78%) |
| **專案視圖 (`project`)** | 302.1 ms | **21.3 ms** | **14.2x** | **6.4 MB** (節省 78%) |
| **衍生索引重建 (`rebuild`)** | N/A | **30.1 ms** | **即時完成** | **2.8 MB** |
| **全量遷移驗證 (`migrate`)** | N/A | **1.27 秒** | **一次性全遷** | 38.2 MB (635 筆事件重構) |

### B. LLM Agent 上下文 Token 減量評測（以 PCNe 專案為例）

```text
[Prompt 上下文注入方案對比]
────────────────────────────────────────────────────────────────────────────────────
1. Naive Flat Dump (原始紀錄+Wiki全文)   ████████████████████ 2,500 tokens (100.0%)
2. 舊版 km project 傾印                  ██████████████████   2,252 tokens (90.1%)
3. 舊版 km graph query 檢索              ██████               739 tokens (29.6%)
4. innen query --q 'PCNe' (BM25+圖鄰接)  ██                   245 tokens (9.8%)  🔥 減量 90.2%
5. innen project nhri-pcne (語意聚合)   ▌                    102 tokens (4.1%)  🔥 減量 95.9%
────────────────────────────────────────────────────────────────────────────────────
```

- **`innen project <id>`（減量 95.9%）**：僅需 102 tokens 即可為 Agent 呈現當前專案完整的 Decisions、Tasks、Experiments 與 Datasets 狀態。
- **`innen query --q <keyword>`（減量 90.2%）**：以 245 tokens 精準涵蓋 13 個核心實體與重要交付物 SHA-256，無任何幻覺或關鍵資訊漏失。

---

## 4. 命令與生命週期

### 日常工作流命令

```bash
# 1. 採集與收錄：多對話逐字稿收集
innen harvest --check

# 2. 檢索與查詢：BM25 + 鄰接子圖展開
innen query --q "智慧藥事照護" --format human

# 3. 專案視圖：結構化匯總決策、任務與實驗
innen project nhri-pcne

# 4. 全文檢索
innen search "PCNe"

# 5. 健康檢查與診斷（嚴格退出碼：0=正常, 1=隔離區異常, 2=日誌/參照損壞）
innen doctor
innen lint

# 6. 索引重建（由 journal 瞬間還原衍生資料庫）
innen rebuild
```

### 關於 `migrate` 指令的生命週期

- **一次性遷移任務**：`innen migrate <old_repo> --out <new_repo>` 為升級過渡設計（具備舊庫唯讀保證、冪等性與缺失參照補全）。
- **遷移後退場**：一旦資料搬遷至 `.innen/journal.jsonl` 且 `innen doctor` 通過驗證（0 quarantined、395 edges all resolve），**`migrate` 指令即完成歷史使命**。日常對話、維護與 Agent 工作流完全由 `harvest`、`ingest`、`query`、`project`、`doctor` 等常態指令接管。

---

## 5. 編譯與安裝

`innen` 為純 Rust 專案，單一靜態二進位檔分發，零外部動態依賴：

```bash
git clone https://github.com/RikaiDev/innen.git
cd innen

# 執行全量測試 (127/127 通過)
cargo test --workspace

# 編譯發行版本 (12MB 單一執行檔)
cargo build --release

# 安裝至本機
cp target/release/innen /usr/local/bin/innen
```

---

## 6. 授權與致敬

- 本實作深受 **Andrej Karpathy** 之「LLM Wiki」設計哲學啟發。
- 專案依循 MIT / Apache-2.0 雙授權。
