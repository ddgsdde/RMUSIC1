export type Schema={type:string;properties?:Record<string,Schema>;required?:string[];items?:Schema;secret?:boolean;description?:string;enum?:unknown[];minimum?:number;maximum?:number;maxLength?:number;maxItems?:number};
export type Values=Record<string,unknown>;
export const object=(value:unknown):value is Values=>!!value&&typeof value==='object'&&!Array.isArray(value);
export const protectedValue=(value:unknown)=>object(value)&&Object.keys(value).length===1&&value.configured===true;
export function mergeValues(values:Values,patch:Values,schema?:Schema):Values {
 const next={...values};for(const [key,value] of Object.entries(patch)){const field=schema?.properties?.[key];next[key]=!field?.secret&&object(value)&&object(next[key])?mergeValues(next[key],value,field):value;}return next;
}
export const initial=(schema:Schema):unknown=>schema.enum?.length?schema.enum[0]:schema.type==='object'?Object.fromEntries((schema.required||[]).filter(name=>schema.properties?.[name]).map(name=>[name,initial(schema.properties![name])])):schema.type==='array'?[]:schema.type==='boolean'?false:schema.type==='string'?'':schema.minimum??0;
const canonical=(value:unknown):string=>JSON.stringify(value,(_,v)=>object(v)?Object.fromEntries(Object.entries(v).sort(([a],[b])=>a.localeCompare(b))):v);
export function validate(value:unknown,schema:Schema,allowProtected=false,path='配置',depth=0):void {
 const fail=(message:string):never=>{throw new Error(`${path}：${message}`);};
 if(depth>8)fail('嵌套超过 Core 上限');
 if(schema.secret&&protectedValue(value)){if(!allowProtected)fail('修改列表后须重新填写受保护字段，尚未发送');return;}
 if(schema.enum&&!schema.enum.some(item=>canonical(item)===canonical(value)))fail('请选择声明中的值');
 switch(schema.type){
 case 'object':{
  if(!object(value)||Object.keys(value).length>32)fail('需要声明的对象');
  const fields=value as Values;
  for(const key of schema.required||[])if(!(key in fields))fail(`缺少 ${key}`);
  for(const [key,item] of Object.entries(fields)){const field=schema.properties?.[key];if(!field)fail(`未声明字段 ${key}`);validate(item,field!,allowProtected,`${path}.${key}`,depth+1);}break;
 }
 case 'array':if(!Array.isArray(value)||value.length>(schema.maxItems??1000))fail('列表超过声明上限');(value as unknown[]).forEach((item,index)=>validate(item,schema.items!,allowProtected,`${path}[${index+1}]`,depth+1));break;
 case 'string':if(typeof value!=='string'||new TextEncoder().encode(value).length>(schema.maxLength??8192))fail('文本超过声明的 UTF-8 长度');break;
 case 'boolean':if(typeof value!=='boolean')fail('需要布尔值');break;
 case 'number':case 'integer':if(typeof value!=='number'||!Number.isFinite(value)||(schema.type==='integer'&&!Number.isSafeInteger(value))||(schema.minimum!==undefined&&(value as number)<schema.minimum)||(schema.maximum!==undefined&&(value as number)>schema.maximum))fail('数值超出声明范围');break;
 default:fail('未声明类型');
 }
}
export function configurationPatch(base:Values,value:Values,schema:Schema):Values {
 validate(value,schema,true);
 function patch(before:unknown,next:unknown,field:Schema,path:string):unknown {
  if(field.secret&&protectedValue(next))return undefined;
  if(field.type==='array'&&canonical(before)!==canonical(next))validate(next,field,false,path);
  if(field.type==='object'&&!field.secret){const result:Values={};for(const [key,item] of Object.entries(next as Values)){const changed=patch(object(before)?before[key]:undefined,item,field.properties![key],`${path}.${key}`);if(changed!==undefined)result[key]=changed;}return Object.keys(result).length?result:undefined;}
  return canonical(before)===canonical(next)?undefined:next;
 }
 const result=(patch(base,value,schema,'配置')||{}) as Values;
 if(new TextEncoder().encode(JSON.stringify(result)).length>65536)throw new Error('插件配置超过 64 KiB，尚未发送');
 return result;
}
