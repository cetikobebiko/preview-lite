"""Lets this throwaway PDFium tree configure with Command Line Tools only.

Edits build/config/apple/sdk_info.py and build/mac/find_sdk.py in place,
adding fallbacks used only when xcodebuild / Xcode platform paths are absent.
"""
import sys
from pathlib import Path

root = Path(sys.argv[1])
CLT = "/Library/Developer/CommandLineTools"

sdk_info = root / "build/config/apple/sdk_info.py"
s = sdk_info.read_text()
old = """  lines = subprocess.check_output(['xcodebuild',
                                   '-version']).decode('UTF-8').splitlines()"""
new = """  try:
    lines = subprocess.check_output(['xcodebuild', '-version'],
                                    stderr=subprocess.DEVNULL
                                    ).decode('UTF-8').splitlines()
  except subprocess.CalledProcessError:
    # Local CLT-only build: report the CLT SDK version as the Xcode version.
    sdk = subprocess.check_output(['xcrun', '--show-sdk-version']
                                  ).decode('UTF-8').strip()
    lines = ['Xcode ' + sdk, 'Build version CLT']"""
assert old in s, "sdk_info xcodebuild block not found"
s = s.replace(old, new, 1)
old = """  settings['sdk_platform_path'] = subprocess.check_output(
      ['xcrun', '-sdk', platform,
       '--show-sdk-platform-path']).decode('UTF-8').strip()"""
new = """  try:
    settings['sdk_platform_path'] = subprocess.check_output(
        ['xcrun', '-sdk', platform, '--show-sdk-platform-path'],
        stderr=subprocess.DEVNULL).decode('UTF-8').strip()
  except subprocess.CalledProcessError:
    settings['sdk_platform_path'] = '%s'""" % CLT
assert old in s, "sdk_info platform block not found"
s = s.replace(old, new, 1)
sdk_info.write_text(s)

find_sdk = root / "build/mac/find_sdk.py"
s = find_sdk.read_text()
old = """  sdk_dir = os.path.join(
      dev_dir, 'Platforms/MacOSX.platform/Developer/SDKs')"""
new = """  sdk_dir = os.path.join(
      dev_dir, 'Platforms/MacOSX.platform/Developer/SDKs')
  if not os.path.isdir(sdk_dir) and dev_dir.endswith('CommandLineTools'):
    sdk_dir = os.path.join(dev_dir, 'SDKs')"""
assert old in s, "find_sdk sdk_dir block not found"
s = s.replace(old, new, 1)
old = """    bin_path = 'Toolchains/XcodeDefault.xctoolchain/usr/bin/'"""
new = """    bin_path = ('usr/bin/' if dev_dir.endswith('CommandLineTools')
                else 'Toolchains/XcodeDefault.xctoolchain/usr/bin/')"""
assert old in s, "find_sdk bin_path block not found"
s = s.replace(old, new, 1)
find_sdk.write_text(s)
print("patched", sdk_info, find_sdk)
