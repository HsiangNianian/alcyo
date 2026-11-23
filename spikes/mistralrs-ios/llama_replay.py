"""Ask a llama-server every request in replay.jsonl, as Alcyo asks it (the GBNF grammar, temperature 0), and write
each answer and time to an output file, so both engines are timed on the same Mac.

    python3 llama_replay.py http://127.0.0.1:18790 replay.jsonl llama-out.jsonl
"""
import json, sys, time, urllib.request

base, src, dst = sys.argv[1:4]
times, same = [], 0
with open(dst, 'w') as out:
    for n, line in enumerate(open(src)):
        item = json.loads(line)
        body = {'model': 'local', 'messages': item['messages'], 'temperature': 0, 'max_tokens': item['max_tokens'], 'grammar': item['gbnf'],
                'cache_prompt': True, 'chat_template_kwargs': {'enable_thinking': False}}
        started = time.time()
        req = urllib.request.Request(base + '/v1/chat/completions', data=json.dumps(body).encode(), headers={'Content-Type': 'application/json'})
        answer = json.load(urllib.request.urlopen(req, timeout=600))['choices'][0]['message']['content']
        took = time.time() - started
        times.append(took)
        same += answer.strip() == item['llama_content'].strip()
        out.write(json.dumps({'n': n, 'llama': answer, 'seconds': took}, ensure_ascii=False) + '\n')
        print(f'{n}: {took:.2f} s', file=sys.stderr)
times.sort()
print(f'{len(times)} requests: p50 {times[len(times)//2]:.2f} s, p95 {times[int(len(times)*.95)]:.2f} s, total {sum(times):.0f} s; same answer as the recording on {same}')
