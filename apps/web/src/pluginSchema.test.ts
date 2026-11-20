import {describe,it,expect} from 'vitest';
import {type Schema,configurationPatch,mergeValues,validate,initial} from './pluginSchema';
describe('跨客户端插件声明与受保护字段',()=>{
 const string:Schema={type:'string',maxLength:8192};
 const secret:Schema={type:'object',secret:true,properties:{key:string},required:['key']};
 const schema:Schema={type:'object',properties:{credential:secret,items:{type:'array',items:secret,maxItems:1000}}};
 it('替换秘密对象时不会合入脱敏标记，也不会误删名为 configured 的真实字段',()=>{
  const base={credential:{configured:true},items:[]};
  const next=mergeValues(base,{credential:{key:'synthetic'}},schema);
  expect(next.credential).toEqual({key:'synthetic'});
  expect(configurationPatch(base,next,schema)).toEqual({credential:{key:'synthetic'}});
  expect(configurationPatch(base,base,schema)).toEqual({});
 });
 it('没有稳定元素身份时，列表变化必须重新填写秘密，禁止错位保留',()=>{
  const base={items:[{configured:true}]};
  expect(configurationPatch(base,base,schema)).toEqual({});
  expect(()=>configurationPatch({items:[]},base,schema)).toThrow('重新填写');
  expect(configurationPatch(base,{items:[{key:'reentered'}]},schema)).toEqual({items:[{key:'reentered'}]});
 });
 it('按 Core 的 UTF8 字节、数组与数值枚举边界检查',()=>{
  expect(()=>validate('中'.repeat(3000),string)).toThrow('UTF-8');
  validate(Array.from({length:1000},()=>''),{type:'array',items:string,maxItems:1000});
  expect(()=>validate(['x'],{type:'array',items:string,maxItems:0})).toThrow();
  expect(initial({type:'integer',enum:[7,8]})).toBe(7);
  expect(()=>validate(3,{type:'integer',enum:[7,8]})).toThrow();
 });
});
