'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('apps/admin/public/monitor.js', 'utf8');
function harness(api) {
  const elements = new Map();
  const element = id => {
    if (!elements.has(id)) elements.set(id, {innerHTML:'',querySelectorAll:()=>[]});
    return elements.get(id);
  };
  const context = {api, $:element, esc:s=>String(s).replaceAll('<','&lt;'), showToast(){}, encodeURIComponent};
  vm.runInNewContext(source.slice(source.indexOf("  let selected ="), source.indexOf('  const tasks =')) + '\nthis.browse=artifacts;this.select=id=>{selected=id;};', context);
  return {context, element};
}
const listing = {root:'builds',path:'',roots:[{id:'builds',name:'构建目录'}],entries:[]};
test('artifact browsing explicitly scopes requests to the drawer project', async () => {
  const calls=[];
  const {context,element}=harness(async (...args)=>{calls.push(args);return listing;});
  context.select('drawer project');
  await context.browse('builds','');
  assert.equal(calls[0][0],'/api/artifacts/list?project=drawer%20project');
  assert.equal(calls[0][2],true);
  assert.match(element('monitorArtifacts').innerHTML,/暂无产物/);
  await element('monitorArtifactOpen').onclick({currentTarget:{}});
  assert.equal(calls[1][0],'/api/artifacts/open-folder?project=drawer%20project');
});
test('stale responses cannot overwrite a newly selected project', async () => {
  const pending=[];
  const {context,element}=harness(()=>new Promise(resolve=>pending.push(resolve)));
  context.select('a'); const first=context.browse();
  context.select('b'); const second=context.browse();
  pending[1]({...listing,path:'b-current'}); await second;
  pending[0]({...listing,path:'a-stale'}); await first;
  assert.match(element('monitorArtifacts').innerHTML,/b-current/);
  assert.doesNotMatch(element('monitorArtifacts').innerHTML,/a-stale/);
});
test('read errors expose retry and reset actions', async () => {
  const {context,element}=harness(async()=>{throw new Error('目录已删除');});
  context.select('a'); await context.browse('builds','missing');
  assert.match(element('monitorArtifacts').innerHTML,/目录已删除/);
  assert.equal(typeof element('monitorArtifactRetry').onclick,'function');
  assert.equal(typeof element('monitorArtifactReset').onclick,'function');
});
