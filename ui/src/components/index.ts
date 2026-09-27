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

export { EmptyState, Notice } from "./feedback";
export { Tabs } from "./tabs";
export type { TabItem, TabTone } from "./tabs";
export { CommandPalette } from "./command-palette";
export type { Command } from "./command-palette";
