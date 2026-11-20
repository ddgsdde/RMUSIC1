export function isCloudSearch(query:string,source:string,albumId:string,artistId:string){
 return Boolean(query.trim()&&source!=="local"&&!albumId&&!artistId);
}
// Search imports emit a library event. Repeating that same search on the event
// would create an unbounded request/import/event loop.
export function requestRevision(query:string,revision:number){return query.trim()?0:revision;}
