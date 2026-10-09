import { useEffect, useState } from "react";

/** 已加载列表的分页：切片、筛选后回首页、数据减少后修正页码。 */
export function useListPagination<T>(rows: readonly T[], resetKey?: string) {
  const [page, setPage] = useState(1);
  const [pageSize, setPageSize] = useState(10);
  const total = rows.length;
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const currentPage = Math.min(page, pageCount);

  useEffect(() => {
    setPage(1);
  }, [resetKey]);

  useEffect(() => {
    setPage((current) => Math.min(current, pageCount));
  }, [pageCount]);

  return {
    items: rows.slice((currentPage - 1) * pageSize, currentPage * pageSize),
    total,
    page: currentPage,
    pageSize,
    onPageChange: setPage,
    onPageSizeChange: (size: number) => {
      setPageSize(size);
      setPage(1);
    }
  };
}
