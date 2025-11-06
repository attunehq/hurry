---
description: Create a git commit with detailed context about the conversation
---

Create a git commit for the current changes using this format:

```
<type>: <brief summary>

## Context

<What problem we were solving or what feature we were building>

## Key Decisions

<Bullet points of important technical choices made during the conversation, including:>
- Specific instructions or constraints I provided
- Alternative approaches we considered and why we chose this one
- Any patterns, style preferences, or architectural decisions
- High-value context about why things are done a certain way

## Prompts & Instructions

<Direct quotes or paraphrases of key prompts I gave you that led to specific outcomes. These should be examples that show effective ways to work with Claude.>

🤖 Generated with [Claude Code](https://claude.com/claude-code)

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Purpose**: These commit messages serve as a learning corpus for others to understand how to effectively collaborate with Claude by seeing concrete examples of what instructions and context lead to good results.

**Note**: The `Co-Authored-By` line must be at the very end with capital A and B to trigger GitHub's Claude icon attribution.

---

## Next Steps

After committing, consider:
- Running `/code-review` if you haven't already reviewed the changes
- Running `/pr-draft` if you're ready to create a pull request
