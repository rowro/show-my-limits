import subprocess,tempfile,os,hashlib,shutil
from pathlib import Path
helper=shutil.which('mitmdump')
assert helper, 'Install mitmdump before running this integration check'
openssl=shutil.which('openssl')
assert openssl, 'Install openssl before running this integration check'
addon=Path(__file__).resolve().parents[1]/'proxy'/'initialize_ca.py'
with tempfile.TemporaryDirectory(prefix='sml-ca-check-') as tmp:
    root=Path(tmp); output=root/'generated'; env=os.environ.copy();env['SML_CA_OUTPUT']=str(output);env['PYTHONDONTWRITEBYTECODE']='1'
    def run(script,bootstrap):
        return subprocess.run([helper,'--mode','regular','--listen-host','127.0.0.1','--listen-port','0','--set',f'confdir={root/bootstrap}','--set','termlog_verbosity=error','-s',str(script)],env=env,capture_output=True,text=True,timeout=25)
    result=run(addon,'bootstrap'); assert result.returncode==0, result.stderr
    certificate=output/'mitmproxy-ca-cert.cer'
    subject=subprocess.run([openssl,'x509','-in',str(certificate),'-noout','-subject'],capture_output=True,text=True,check=True).stdout.strip()
    assert 'limits-mitmproxy' in subject
    digest=hashlib.sha256((output/'mitmproxy-ca.pem').read_bytes()).digest()
    repeat=run(addon,'bootstrap-repeat'); assert repeat.returncode!=0
    assert hashlib.sha256((output/'mitmproxy-ca.pem').read_bytes()).digest()==digest
    verify=root/'verify_store.py'
    verify.write_text('from pathlib import Path\nfrom mitmproxy import certs,ctx\nimport os\ndef load(loader):\n p=Path(os.environ["SML_CA_OUTPUT"])\n store=certs.CertStore.from_store(p,"mitmproxy",2048)\n leaf=store.get_cert("example.test",["example.test"])\n (p/"test-leaf.pem").write_bytes(leaf.cert.to_pem())\ndef running():\n ctx.master.shutdown()\n')
    result=run(verify,'bootstrap-verify'); assert result.returncode==0, result.stderr
    subprocess.run([openssl,'verify','-CAfile',str(certificate),str(output/'test-leaf.pem')],capture_output=True,text=True,check=True)
    if os.name=='posix': assert ((output/'mitmproxy-ca.pem').stat().st_mode & 0o077)==0
    print(subject)
    print('PASS: mitmproxy loads CA, signs a verifiable leaf, keeps private key restricted, refuses to overwrite existing CA')
