'use strict';
const fs = require('fs');
const path = require('path');
const { spawn, execFileSync } = require('child_process');
const mode = ['build', 'check', 'test'].includes(process.argv[2]) ? process.argv[2] : 'dev';
const root = path.resolve(__dirname, '..');
let command;
let args;
if (process.platform === 'win32') {
  let install = process.env.VSINSTALLDIR;
  const vswhere = path.join(process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)', 'Microsoft Visual Studio', 'Installer', 'vswhere.exe');
  if (!install && fs.existsSync(vswhere)) {
    try { install = execFileSync(vswhere, ['-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64', '-property', 'installationPath'], { encoding: 'utf8', windowsHide: true }).trim(); } catch {}
  }
  // Support this machine's custom installation even if installer discovery is unavailable.
  if (!install && fs.existsSync('D:\\VSBuildTools\\Common7\\Tools\\VsDevCmd.bat')) install = 'D:\\VSBuildTools';
  const developer = install && path.join(install, 'Common7', 'Tools', 'VsDevCmd.bat');
  command = process.env.ComSpec || 'cmd.exe';
  const npm = `npm --prefix apps/admin run ${mode}`;
  args = ['/d', '/s', '/c', developer && fs.existsSync(developer) ? `call "${developer}" -arch=x64 -host_arch=x64 && ${npm}` : npm];
} else {
  command = 'npm';
  args = ['--prefix', 'apps/admin', 'run', mode];
}
const child = spawn(command, args, { cwd: root, stdio: 'inherit', windowsVerbatimArguments: process.platform === 'win32' });
child.on('error', error => { console.error(error.message); process.exitCode = 1; });
child.on('exit', code => { process.exitCode = code ?? 1; });
