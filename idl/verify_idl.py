import json,urllib.request,time,hashlib,sys
RPC="https://api.mainnet-beta.solana.com"
B58='123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz'
def b58d(s):
    n=0
    for c in s: n=n*58+B58.index(c)
    out=n.to_bytes((n.bit_length()+7)//8,'big') if n else b''
    return b'\0'*(len(s)-len(s.lstrip('1')))+out
def rpc(m,p):
    for i in range(6):
        try:
            r=urllib.request.Request(RPC,data=json.dumps({"jsonrpc":"2.0","id":1,"method":m,"params":p}).encode(),headers={'content-type':'application/json'})
            d=json.load(urllib.request.urlopen(r,timeout=30))
            if 'error' in d: 
                if d['error'].get('code')==429: time.sleep(2*(i+1)); continue
                return None
            return d['result']
        except Exception as e:
            time.sleep(2*(i+1))
    return None
EVENT_IX=bytes.fromhex('e445a52e51cb9a1d')
progs={'pumpfun':'6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P','pumpswap':'pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA','launchlab':'LanMV9sAd7wArD4vJFi2qDdfnVhFxYSUg6eADduJ3uj','cpmm':'CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C','dbc':'dbcij3LWUppWqq96dh6gJWwBifmcGfLSB5D4DuSMaqN','damm2':'cpamdpZCGKUy5JxQXB4dcpGPiikHawvSWAd6mEn1sGG'}
report={}
for name,pid in progs.items():
    idl=json.load(open(f'idl/{name}.json'))
    ixd={bytes(i['discriminator']).hex():i['name'] for i in idl['instructions'] if 'discriminator' in i}
    evd={bytes(e['discriminator']).hex():e['name'] for e in idl.get('events',[]) if 'discriminator' in e}
    if not ixd:  # legacy IDL: compute sha256("global:name")[:8]
        ixd={hashlib.sha256(f"global:{i['name']}".encode()).digest()[:8].hex():i['name'] for i in idl['instructions']}
        evd={hashlib.sha256(f"event:{e['name']}".encode()).digest()[:8].hex():e['name'] for e in idl.get('events',[])}
    sigs=rpc('getSignaturesForAddress',[pid,{"limit":40}]) or []
    seen_ix={}; seen_ev={}; unknown_ix=0; checked=0; used=[]
    for s in sigs:
        if s.get('err'): continue
        tx=rpc('getTransaction',[s['signature'],{"encoding":"json","maxSupportedTransactionVersion":0}])
        if not tx: continue
        checked+=1; used.append(s['signature'])
        msg=tx['transaction']['message']; keys=list(msg['accountKeys'])
        la=(tx['meta'] or {}).get('loadedAddresses') or {}
        keys+=la.get('writable',[])+la.get('readonly',[])
        allix=[(ix,None) for ix in msg['instructions']]
        for inner in (tx['meta'] or {}).get('innerInstructions',[]) or []:
            allix+=[(ix,inner['index']) for ix in inner['instructions']]
        for ix,parent in allix:
            if keys[ix['programIdIndex']]!=pid: continue
            data=b58d(ix['data'])
            if data[:8]==EVENT_IX:
                ev=evd.get(data[8:16].hex()); seen_ev[ev or 'UNKNOWN '+data[8:16].hex()]=seen_ev.get(ev or 'UNKNOWN '+data[8:16].hex(),0)+1
            else:
                nm=ixd.get(data[:8].hex())
                if nm: seen_ix[nm]=seen_ix.get(nm,0)+1
                else: unknown_ix+=1
        time.sleep(0.4)
        if checked>=8: break
    report[name]={'program':pid,'tx_checked':checked,'instructions_matched':seen_ix,'events_matched':seen_ev,'unknown_instructions':unknown_ix,'sample_sigs':used[:2]}
    print(f"\n== {name} {pid}\n  tx checked: {checked}\n  instructions matched: {seen_ix}\n  events matched: {seen_ev}\n  unknown instruction discriminators: {unknown_ix}\n  sample: {used[:2]}")
    time.sleep(1)
json.dump(report,open('idl-verify.json','w'),indent=1)
