{{#if args}}
The user's stated intention for these changes is:

{{args}}

Proceed using this intention as context.
{{else}}
First, examine the conversation history and git diff to understand what changes were made and why.

Based on your analysis, state what you believe the intention was and ask the user to confirm:

"Based on the changes, I believe the intention was: [your inference]. Is that correct? If not, please tell me the actual intention."

Wait for the user's response before proceeding. If they provide a different intention, use that. If they confirm, proceed with your inferred intention.
{{/if}}
