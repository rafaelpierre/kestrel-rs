import argparse,pathlib,json,subprocess,time,os,datetime,hashlib
parser=argparse.ArgumentParser(description='Matched default/portable query replay; outputs are observations, not relevance scores.')
parser.add_argument('--binary',type=pathlib.Path,required=True)
parser.add_argument('--output',type=pathlib.Path,required=True)
parser.add_argument('--stage',choices=['initial','recovery'],required=True)
options=parser.parse_args()
p=options.output.resolve()
b=str(options.binary.resolve())
stage=options.stage
if stage=='initial': p.mkdir(parents=True,exist_ok=False)
else:
 metadata=json.loads((p/'metadata.json').read_text())
 if metadata['sha256']!=hashlib.sha256(pathlib.Path(b).read_bytes()).hexdigest():
  raise SystemExit('Recovery binary differs from initial binary')
 if any(json.loads(line)['id'].endswith('-s2') for line in (p/'calls.jsonl').read_text().splitlines()):
  raise SystemExit('Recovery already started; choose a fresh batch')
if stage=='initial':
 (p/'metadata.json').write_text(json.dumps({'binary':b,'sha256':hashlib.sha256(pathlib.Path(b).read_bytes()).hexdigest(),'version':subprocess.check_output([b,'--version'],text=True).strip(),'comparison':'same binary, alternating native default and explicit portable; exact historical flags; live uncontrolled providers; diagnostics enabled','started':datetime.datetime.now(datetime.timezone.utc).isoformat()},indent=2))
cases=pathlib.Path(__file__).parent/(stage+'.json')
(p/(stage+'.json')).write_bytes(cases.read_bytes())
jobs=json.loads(cases.read_text())
for i,job in enumerate(jobs):
 modes=['default','portable'] if i%2==0 else ['portable','default']
 for mode in modes:
  label=mode+'-'+job['id']; args=[b]+job['args']+(['--query-syntax','portable'] if mode=='portable' else [])
  env=os.environ.copy();env.update(KESTRELSEARCH_BENCHMARK_ARTIFACT_DIR=str(p),KESTRELSEARCH_BENCHMARK_RUN_ID=label)
  start=time.monotonic(); timestamp=datetime.datetime.now(datetime.timezone.utc).isoformat()
  try:
   proc=subprocess.run(args,capture_output=True,text=True,env=env,timeout=30); code=proc.returncode;stdout=proc.stdout;stderr=proc.stderr
  except subprocess.TimeoutExpired:
   code=124;stdout='';stderr='runner timeout 30s'
  record={'id':label,'args':args,'started_utc':timestamp,'wall_ms':(time.monotonic()-start)*1000,'exit_code':code,'stdout':stdout,'stderr':stderr}
  with (p/'calls.jsonl').open('a') as f:f.write(json.dumps(record)+'\n')
  try:data=json.loads(stdout)
  except ValueError:data={}
  print(label,round(record['wall_ms']),code,flush=True)
  for r in data.get('results',[]):print(json.dumps({k:r.get(k) for k in ['title','url','snippet','engine']}),flush=True)
