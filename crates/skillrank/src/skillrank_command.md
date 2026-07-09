---
description: Find, inspect, install, and evaluate agent skills with skillrank
argument-hint: "[recommend | search <query> | install <slug> | list]"
allowed-tools: Bash(skillrank:*)
---

Run skillrank for the current repository with the arguments the user gave:

!`skillrank $ARGUMENTS`

If the user gave no arguments (the command above printed usage), run
`skillrank recommend` to suggest skills for this repo's stack. Otherwise the
arguments were passed straight through — common ones are `search <query>`,
`install <slug>`, `list`, `show <slug>`, and `recommend`.

Present the results concisely. When results include installable skills, offer to
install the best match, then run `skillrank install <slug>`.
