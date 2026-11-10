import { useCallback, useEffect, useRef, useState } from "react";

/** 极简异步加载 hook：返回数据、加载态、错误与手动重载。 */
export function useAsync<T>(fn: () => Promise<T>, deps: unknown[] = []) {
  const [data, setData] = useState<T | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const alive = useRef(true);
  const fnRef = useRef(fn);
  fnRef.current = fn;

  const run = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const res = await fnRef.current();
      if (alive.current) setData(res);
    } catch (e) {
      if (alive.current) setError((e as Error).message || "加载失败");
    } finally {
      if (alive.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    run();
    return () => { alive.current = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  return { data, loading, error, reload: run, setData };
}

/** 分页列表：滚动到底自动加载下一页。 */
export function usePaged<T>(fetcher: (page: number) => Promise<{ items: T[]; pagination?: { pages: number } }>, deps: unknown[] = []) {
  const [items, setItems] = useState<T[]>([]);
  const [page, setPage] = useState(0);
  const [pages, setPages] = useState(1);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const busy = useRef(false);
  const fetcherRef = useRef(fetcher);
  fetcherRef.current = fetcher;

  const loadMore = useCallback(async () => {
    if (busy.current || page >= pages) return;
    busy.current = true;
    setLoading(true);
    try {
      const next = page + 1;
      const res = await fetcherRef.current(next);
      setItems((prev) => {
        const seen = new Set(prev.map((x) => (x as { id: number }).id));
        return [...prev, ...(res.items || []).filter((x) => !seen.has((x as { id: number }).id))];
      });
      setPage(next);
      setPages(res.pagination?.pages || next);
      setError(null);
    } catch (e) {
      setError((e as Error).message || "加载失败");
    } finally {
      busy.current = false;
      setLoading(false);
    }
  }, [page, pages]);

  const reset = useCallback(() => {
    setItems([]);
    setPage(0);
    setPages(1);
    setError(null);
  }, []);

  useEffect(() => {
    reset();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, deps);

  useEffect(() => {
    if (page === 0) loadMore();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page]);

  return { items, loading, error, loadMore, hasMore: page < pages, reset };
}