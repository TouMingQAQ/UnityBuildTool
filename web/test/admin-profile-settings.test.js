'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('apps/admin/public/app.js', 'utf8');
function harness(api) {
  const elements = new Map();
  const $ = id => {
    if (!elements.has(id)) elements.set(id, {value:'',disabled:false,textContent:''});
    return elements.get(id);
  };
  $('pcProfileSelect').value = '__project_default__'; $('projectPath').value = 'game';
  const state = {activeProjectId:'a',scannedProjectId:'a',scannedProjectPath:'game',profiles:[{assetPath:'__project_default__'}]};
  const context = {$, state, api, flushProjectSaves:async()=>{}, syncBuildNodesWithProfiles(){}, updateModalPlatformUI(){}, scheduleSave(){}, showToast(){}};
  vm.runInNewContext(source.slice(source.indexOf('let playerSettingsRequest ='), source.indexOf('function openBuildNodeModal(')) + '\nthis.load=loadProfilePlayerSettings;this.save=saveProfilePlayerSettings;', context);
  return {context,$,state};
}
const settings = {editable:true,revision:'r1',fallback:true,path:'game/ProjectSettings/ProjectSettings.asset',productName:'Game',companyName:'Studio',bundleVersion:'1.0'};
test('legacy settings load and save three fields with the file revision', async()=>{
  const calls=[];
  const {context,$,state}=harness(async(route,body)=>{calls.push({route,body});return {...settings,companyName:body.companyName??settings.companyName,revision:calls.length===1?'r1':'r2'};});
  await context.load();
  assert.equal($('pcCompanyName').value,'Studio');
  assert.match($('pcPlayerSettingsInfo').textContent,/项目默认设置/);
  $('pcCompanyName').value='新公司'; await context.save();
  assert.equal(calls[1].route,'/api/profile-settings/write');
  assert.equal(calls[1].body.assetPath,'__project_default__');
  assert.equal(calls[1].body.revision,'r1');
  assert.equal(state.profiles[0].companyName,'新公司');
  assert.match($('pcPlayerSettingsInfo').textContent,/已保存/);
});
test('switching profiles during a read ignores the previous response',async()=>{
  const pending=[];
  const {context,$}=harness(()=>new Promise(resolve=>pending.push(resolve)));
  const first=context.load(); await new Promise(setImmediate);
  $('pcProfileSelect').value='Assets/Other.asset';
  const second=context.load(); await new Promise(setImmediate);
  pending[1]({...settings,productName:'Current'}); await second;
  pending[0]({...settings,productName:'Stale'}); await first;
  assert.equal($('pcProductName').value,'Current');
});
test('unsupported serialization keeps save disabled while showing inherited values',async()=>{
  const {context,$}=harness(async()=>({...settings,editable:false,inherited:true,fallback:false}));
  await context.load();
  assert.equal($('pcPlayerSave').disabled,true);
  assert.equal($('pcProductName').value,'Game');
  assert.match($('pcPlayerSettingsInfo').textContent,/Unity 中启用覆盖/);
});
