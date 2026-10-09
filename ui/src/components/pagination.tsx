import { Pager } from "./pager";
import { Select } from "./primitives";

/** 通用列表分页栏；数据查询或切片由调用方负责。页码从 1 起。 */
export function Pagination({
  total,
  page,
  pageSize,
  onPageChange,
  onPageSizeChange,
  disabled = false,
  label = "分页"
}: {
  total: number;
  page: number;
  pageSize: number;
  onPageChange: (page: number) => void;
  onPageSizeChange: (pageSize: number) => void;
  disabled?: boolean;
  label?: string;
}) {
  const pageCount = Math.max(1, Math.ceil(total / pageSize));
  const currentPage = Math.max(1, Math.min(page, pageCount));
  const first = total === 0 ? 0 : (currentPage - 1) * pageSize + 1;
  const last = Math.min(currentPage * pageSize, total);

  return (
    <nav className="pagination" aria-label={label}>
      <div className="pagination-summary">
        <span className="row-meta">共 {total} 条 · {first}–{last}</span>
        <Select
          aria-label="每页数量"
          value={pageSize}
          disabled={disabled}
          onChange={(event) => {
            onPageChange(1);
            onPageSizeChange(Number(event.target.value));
          }}
        >
          {[10, 20, 50].map((size) => <option key={size} value={size}>{size} 条/页</option>)}
        </Select>
      </div>
      <div className="pagination-controls">
        <Pager
          page={currentPage}
          pageCount={pageCount}
          onPage={onPageChange}
          disabled={disabled}
          withLast
          ariaLive
        />
      </div>
    </nav>
  );
}
