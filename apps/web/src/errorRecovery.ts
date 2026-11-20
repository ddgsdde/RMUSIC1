/** Keep recoverable network failures actionable and consistent across editor workflows. */
export function errorMessage(cause:unknown):string {
 const message=cause instanceof Error?cause.message:typeof cause==='string'?cause:'操作失败，请重试';
 if(/Failed to fetch|NetworkError|Load failed|fetch failed|error sending request|ERR_(?:CONNECTION|NETWORK|INTERNET)/i.test(message))return '无法连接核心，请检查网络或等待重连后重试';
 if(cause instanceof Error&&cause.name==='AbortError')return '操作已取消，请重试';
 return message;
}
