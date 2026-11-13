'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('apps/admin/public/app.js', 'utf8');
function harness(api) {
  const refresh = {};
  const button = {dataset:{projectPath:'0'}};
  const host = {innerHTML:'',querySelector:()=>refresh,querySelectorAll:()=>[button]};
  const context = {api, state:{activeProjectId:'settings-project'}, esc:String, showToast(){}, flushProjectSaves:async()=>{}};
  vm.runInNewContext(source.slice(source.indexOf('async function renderProjectPaths('), source.indexOf('async function readRootConfig(')) + '\nthis.render=renderProjectPaths;', context);
  return {context, host, button, refresh};
}
const listing = {paths:[{id:'project',name:'工程根目录',path:'C:/Unity/Game',exists:true}]};
test('path buttons open the displayed project even when settings selects another project', async () => {
  const calls=[];
  const {context,host,button}=harness(async(...args)=>{calls.push(args);return listing;});
  await context.render(host,'drawer project');
  await button.onclick();
  assert.equal(calls[0][0],'/api/project-paths?project=drawer%20project');
  assert.equal(calls[1][0],'/api/project-paths/open?project=drawer%20project');
  assert.equal(calls[1][1].id,'project');
  assert.equal(calls[1][2],true);
});
test('path rendering ignores responses from the previously selected project', async () => {
  const pending=[];
  const {context,host}=harness(()=>new Promise(resolve=>pending.push(resolve)));
  const first=context.render(host,'a'), second=context.render(host,'b');
  pending[1]({paths:[{...listing.paths[0],path:'b-current'}]}); await second;
  pending[0]({paths:[{...listing.paths[0],path:'a-stale'}]}); await first;
  assert.match(host.innerHTML,/b-current/);
  assert.doesNotMatch(host.innerHTML,/a-stale/);
});
test('missing paths remain visible with a disabled open button', async () => {
  const {context,host}=harness(async()=>({paths:[{...listing.paths[0],exists:false}]}));
  await context.render(host,'a');
  assert.match(host.innerHTML,/目录尚未生成或已移除/);
  assert.match(host.innerHTML,/data-project-path="0" disabled/);
});
