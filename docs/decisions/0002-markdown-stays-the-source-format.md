---
status: accepted
date: 2026-10-17
---

# 0002 Markdown stays the source format

> Short version: agents keep writing plain Markdown. driftmark does not invent a new format. It asks for a small set of conventions that a lint checks, so pages render well and agents stay cheap.

## Situation

Agents read and write the documents many times a day, so every extra token costs money and context. Humans need the same documents to render well. The question was whether HTML, a structured format or a new format would serve agents better than Markdown.

## Evidence

**Tokens, OpenAI tokenizer (`o200k_base`), four real documents, 47 KB:**

| Format | Tokens compared to Markdown |
|---|---|
| Plain text | 0.94 (loses headings and tables) |
| Markdown | 1.00 |
| AsciiDoc | 1.00 |
| YAML | 1.20 |
| XML | 1.27 |
| HTML | 1.29 to 1.30 |
| JSON, pretty-printed | 1.37 |

**Claude's own tokenizer and accuracy, 20 real documents, 100 questions per format per model, Sonnet 5.5 and Opus 5.5, run on 2026-10-17:**

| Format | Correct answers | Tokens compared to Markdown |
|---|---|---|
| Markdown | 100% | 1.00 |
| Markdown with XML section tags | 100% | 1.03 |
| AsciiDoc | 100% | 1.05 |
| HTML | 100% | 1.36 |

The questions were too easy to separate the formats on accuracy. The test rules out a large loss for any format. It cannot detect small differences on harder tasks.

**Editing safety.** One random line was deleted 400 times per format. JSON stopped parsing in 72% of tries and XML in 56%. Markdown never stopped parsing; the typical damage was 0.5% of the rendered page. The worst Markdown cases were a deleted code-fence line.

**What vendors chose.** Claude Code skills, [AGENTS.md](https://agents.md/), [llms.txt](https://llmstxt.org/), and Cloudflare and Vercel serving Markdown to agents ([Cloudflare](https://blog.cloudflare.com/markdown-for-agents/)) all use Markdown.

**Comprehension studies** find no format that wins on every model ([He et al. 2024](https://arxiv.org/html/2411.10541v1), [Sclar et al. 2024](https://arxiv.org/abs/2310.11324)).

## Decision

Markdown (CommonMark plus GitHub extensions) stays the source. driftmark recommends, and its lint checks, these conventions:

1. Metadata such as status, date and supersedes sits in YAML frontmatter. driftmark also reads a `> Status:` line under the title, because existing corpora use it.
2. One `#` title, then a short summary, then `##` sections. No skipped heading levels.
3. Every code block names its language and is closed.
4. Mermaid is the default diagram language. D2 is allowed for architecture pictures.
5. Raw HTML only from a short allowed list, such as `<details>`.
6. Callouts use `> [!NOTE]`-style alerts.
7. Links are relative Markdown links, never wiki links.

Rendering never needs syntax that looks broken on GitHub or GitLab.

## What this costs

- Nothing for agents. The conventions add almost no tokens.
- The lint must exist and run where agents work (a hook) and in CI. Written rules alone do not hold.
