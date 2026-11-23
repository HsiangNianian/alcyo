"""Turn requests recorded from Alcyo's board (a proxy in front of llama-server) into replay.jsonl: each request's
messages, its board grammar converted to Lark for mistral.rs (checked by llguidance), and llama-server's answer.

    python3 prepare.py requests.jsonl replay.jsonl      # needs `pip install llguidance`
"""
import json, sys
import llguidance as L
from llguidance.gbnf_to_lark import gbnf_to_lark

rows = [json.loads(line) for line in open(sys.argv[1])]
kept = 0
with open(sys.argv[2], 'w') as out:
    for r in rows:
        req = r['request']
        if 'grammar' not in req:
            continue
        lark = gbnf_to_lark(req['grammar'])
        problem = L.LLMatcher.validate_grammar(L.grammar_from('lark', lark))
        if problem:
            print('skipped, grammar not accepted:', problem, file=sys.stderr)
            continue
        out.write(json.dumps({
            'messages': req['messages'], 'max_tokens': req.get('max_tokens', 160), 'gbnf': req['grammar'], 'lark': lark,
            'llama_content': r['reply']['choices'][0]['message']['content'], 'llama_seconds': r['seconds'],
        }, ensure_ascii=False) + '\n')
        kept += 1
print(f'{kept} of {len(rows)} requests kept')
