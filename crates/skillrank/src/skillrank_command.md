---
description: Find, inspect, install, and evaluate agent skills with skillrank
argument-hint: "[recommend | search <query> | install <slug> | show <slug> | eval <slug> | list]"
allowed-tools: Bash(skillrank:*)
---

Run skillrank for the current repository with the arguments the user gave:

!`skillrank $ARGUMENTS`

If the user gave **no arguments**, the command above printed skillrank's usage
menu (the ~14 things it can do). Show that menu briefly, then immediately run
`skillrank recommend` to suggest skills for this repo's detected stack — so the
user sees both what's available and a concrete recommendation.

Otherwise the arguments were passed straight through — common ones are
`search <query>`, `install <slug>`, `show <slug>`, `eval <slug>`, `list`, and
`recommend`.

Present the results concisely. When results include installable skills, offer to
install the best match, then run `skillrank install <slug>` (it becomes active
for the agent in this repo immediately — no restart needed).
