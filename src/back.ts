/** 全局返回拦截：面板等临时层优先消费，未消费则走导航栈。 */
type Handler = () => boolean;

let handler: Handler | null = null;

export function setBackHandler(h: Handler | null) {
  handler = h;
}

/** 返回 true 表示已被消费（不再退栈） */
export function runBack(): boolean {
  try {
    return handler ? handler() : false;
  } catch {
    return false;
  }
}