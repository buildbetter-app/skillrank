#!/usr/bin/env bash
python3 - <<'PYEOF'
from pathlib import Path
import re, sys
path=Path('tasks/marketing-ideas/marketing-plan.md'); raw=path.read_text(errors='ignore') if path.is_file() else ''; low=raw.lower(); errors=[]
heads=list(re.finditer(r'^#{2,3}\s+(?:(?:idea|play|experiment)\s*)?(?:[1-5][.):-]?\s*)?(.+)$',raw,re.I|re.M))
reserved=re.compile(r'30[- ]day|sequence|budget|summary|overview',re.I)
ideas=[h for h in heads if not reserved.search(h.group(1))]
if len(ideas)!=5: errors.append(f'plan must contain exactly five idea sections; found {len(ideas)}')
for i,h in enumerate(ideas):
    end=ideas[i+1].start() if i+1<len(ideas) else (next((x.start() for x in heads if x.start()>h.start() and reserved.search(x.group(1))),len(raw)))
    section=raw[h.start():end].lower()
    fields=[('channel',r'\bchannel\b'),('hook/message',r'\b(?:hook|message)\b'),('rationale',r'\b(?:why|rationale)\b'),('cash cost',r'\b(?:cash\s+)?cost\b'),('founder time',r'\bfounder\s+time\b'),('success metric',r'\b(?:success\s+metric|metric|kpi)\b'),('first test',r'\b(?:first|smallest|initial)\s+test\b')]
    for label,pat in fields:
        if not re.search(pat,section): errors.append(f'idea {i+1} misses {label}')
if not re.search(r'30[- ]day',low) or not re.search(r'\b(?:day|week)\s*[1-4]\b',low): errors.append('missing concrete 30-day sequence')
budget_section=re.split(r'^#{2,3}\s+.*budget.*$',raw,flags=re.I|re.M)
budget=budget_section[-1] if len(budget_section)>1 else ''
amounts=[float(x.replace(',','')) for x in re.findall(r'\$\s*([0-9][0-9,]*(?:\.\d{1,2})?)',budget)]
declared=re.search(r'\btotal\b[^\n$]*\$\s*([0-9][0-9,]*(?:\.\d{1,2})?)',budget,re.I)
if not budget or not declared: errors.append('budget section needs an explicit total')
else:
    total=float(declared.group(1).replace(',',''))
    line_amounts=[float(x.replace(',','')) for line in budget.splitlines() if 'total' not in line.lower() for x in re.findall(r'\$\s*([0-9][0-9,]*(?:\.\d{1,2})?)',line)]
    if total>1000: errors.append(f'budget exceeds $1,000: ${total:g}')
    if line_amounts and abs(sum(line_amounts)-total)>0.01: errors.append(f'budget line items sum to ${sum(line_amounts):g}, not ${total:g}')
for phrase in ['12 hours','free trial']:
    if phrase not in low: errors.append(f'plan does not account for {phrase}')
for pattern in [r'(?<!no )(?<!without an )\bexisting audience\b',r'\bour customers say\b',r'\bguaranteed\b',r'\bindustry-leading\b']:
    if re.search(pattern,low): errors.append(f'unsupported claim detected: {pattern}')
for e in errors: print('FAIL',e)
sys.exit(0 if not errors else 1)
PYEOF
