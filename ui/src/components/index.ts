/**
 * 组件层出口。
 *
 * 统一从 `../components` 导入，而不是逐个文件——这样将来调整文件划分时，
 * 调用方不需要跟着改。
 */

export {
  Button,
  ChipGroup,
  Field,
  SearchInput,
  Select,
  StatusPill,
  TextInput,
  Toolbar,
  ToolbarSpacer
} from "./primitives";
export type { ButtonVariant } from "./primitives";

export { EmptyState, HintBar, Notice } from "./feedback";

/* 通用构件：弹层、加载态、卡片、列表。四者都在**多处**被用到，
 * 之前每一处各抄一遍骨架，是走样的来源。 */
export { Dialog } from "./dialog";
export { hasLayerAbove } from "./overlay-stack";
export { LoadingBlock, Spinner } from "./loading";
export { Card, DescriptionList } from "./card";
export { List, ListRow, MetaItem } from "./list";
export type { ListRowProps } from "./list";
export { useAsyncData, useToast } from "./use-async-data";

/* 设计系统构件：图标、徽章、状态点、进度条、开关、浮层。 */
export * from "./icon";
export * from "./badge";
export * from "./status-dot";
export * from "./progress";
export * from "./toggle";
export * from "./popover";
