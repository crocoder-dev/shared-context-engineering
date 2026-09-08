import subprocess

base = '91fc89b70ff5e6a630ec8ae5df18ec12ce079757'
implementation = '164f2b6e74c16cebe4ea466b5ee08a7846d9782d'
pr_branch = 'codex/compress-workflow-preambles'

subprocess.run(['git', 'config', 'user.name', 'David Abram'], check=True)
subprocess.run(['git', 'config', 'user.email', 'david@crocoder.dev'], check=True)
subprocess.run(['git', 'checkout', '--detach', base], check=True)
subprocess.run(['git', 'cherry-pick', implementation], check=True)
subprocess.run(['git', 'push', '--force-with-lease', 'origin', f'HEAD:{pr_branch}'], check=True)
print(subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True).strip())
