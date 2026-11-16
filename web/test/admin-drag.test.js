'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const source = fs.readFileSync('apps/admin/public/app.js', 'utf8');
function load(name, context) {
  const start = source.indexOf(`function ${name}(`);
  const end = source.indexOf('\nfunction ', start + 1);
  vm.runInNewContext(source.slice(start, end < 0 ? undefined : end), context);
  return context[name];
}
for (const kind of ['build', 'vcs']) {
  test(`${kind} nodes move between groups and retain explicit order`, () => {
    const nodes = [{id:'a'}, {id:'b'}, {id:'c'}];
    const groups = [{id:'g', nodeIds:['a','b']}, {id:'h', nodeIds:[]}];
    const state = {[`${kind}Nodes`]:nodes, [`${kind}Groups`]:groups};
    const context = {state, scheduleSave(){}, scheduleVcsSave(){}, renderBuildGroups(){}, renderVcs(){}, showToast(){}};
    context[`${kind}NodeById`] = id => state[`${kind}Nodes`].find(n => n.id === id);
    context[`${kind}GroupById`] = id => groups.find(g => g.id === id);
    context.vcsGroupOf = id => groups.find(g => g.nodeIds.includes(id));
    const move = load(kind === 'build' ? 'moveBuildNodeToGroup' : 'moveVcsNodeToGroup', context);
    move('b','g','a');
    assert.equal(groups[0].nodeIds.join(','),'b,a');
    move('b','h');
    assert.equal(groups[0].nodeIds.join(','),'a');
    assert.equal(groups[1].nodeIds.join(','),'b');
    move('b',null,'c');
    assert.equal(groups[1].nodeIds.length,0);
    assert.equal(state[`${kind}Nodes`].map(n=>n.id).join(','),'a,b,c');
    move('b',null,'a');
    assert.equal(state[`${kind}Nodes`].map(n=>n.id).join(','),'b,a,c');
    move('b',null,'b');
    assert.equal(state[`${kind}Nodes`].map(n=>n.id).join(','),'b,a,c');
  });
}
test('new build nodes only join the explicitly selected group', () => {
  const fields = new Proxy({}, {get:(_,id) => ({value:id==='pcProfileSelect'?'profile':'',checked:false,classList:{add(){}}})});
  const state = {profiles:[], buildNodes:[], buildGroups:[{id:'g',nodeIds:[]}], activeBuildGroupId:'g',pcVcsGroups:[]};
  const context = {state,$:id=>fields[id],generateId:()=>`node${state.buildNodes.length}`,buildGroupById:id=>state.buildGroups.find(g=>g.id===id),showToast(){},renderBuildGroups(){},scheduleSave(){}};
  const save = load('saveBuildNodeModal',context);
  save();
  assert.equal(state.buildNodes.length,1);
  assert.equal(state.buildGroups[0].nodeIds.length,0);
  state.buildEditTargetGroupId='g';
  save();
  assert.equal(state.buildGroups[0].nodeIds.join(','),'node1');
});
