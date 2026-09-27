import { useEffect, useRef, useState } from "react";

import type { AppMenu, AppMenuItem } from "./app-menus";

/**
 * 菜单栏。
 *
 * # 行为与旧平台一致的部分
 *
 * - 横向四个菜单，**点击**展开（不是悬停展开——悬停会让鼠标路过就弹开）；
 * - 已经展开时，鼠标移到另一个菜单上直接切过去（旧平台也是这样，免得再点一次）；
 * - 子菜单（「导出」）在项上**点击**展开；
 * - 点外面关闭、按 Escape 关闭；
 * - 分隔符不响应点击；`danger` 的项用醒目样式，禁用的项不可点。
 *
 * # 快捷键只显示不绑定
 *
 * 「Ctrl + K」这类提示由菜单显示，但按键绑定在别处（`app.tsx` 的全局监听）。
 * 在菜单里再绑一次会出现「同一个键两个地方处理」，那是 bug 的温床。
 */
export function AppMenuBar({ menus }: { menus: AppMenu[] }) {
  const [openId, setOpenId] = useState<string | null>(null);
  /** 当前展开的子菜单项的标签；`null` 表示没有。 */
  const [openChild, setOpenChild] = useState<string | null>(null);
  const rootRef = useRef<HTMLDivElement | null>(null);

  useEffect(() => {
    if (!openId) return;
    const onPointerDown = (event: PointerEvent) => {
      if (rootRef.current && !rootRef.current.contains(event.target as Node)) {
        setOpenId(null);
        setOpenChild(null);
      }
    };
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        setOpenId(null);
        setOpenChild(null);
      }
    };
    document.addEventListener("pointerdown", onPointerDown);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      document.removeEventListener("pointerdown", onPointerDown);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, [openId]);

  /** 执行一项，然后关掉整个菜单。禁用项不执行。 */
  const run = (item: AppMenuItem) => {
    if (item.disabled || !item.action) return;
    setOpenId(null);
    setOpenChild(null);
    item.action();
  };

  const renderItem = (item: AppMenuItem, key: string) => {
    if (item.separator) {
      return <div key={key} className="menu-separator" role="separator" />;
    }
    if (item.children?.length) {
      const expanded = openChild === item.label;
      return (
        <div key={key} className="menu-item-wrap">
          <button
            type="button"
            className="menu-item has-children"
            disabled={item.disabled}
            aria-haspopup="menu"
            aria-expanded={expanded}
            onClick={() => setOpenChild(expanded ? null : item.label)}
          >
            <span>{item.label}</span>
            <span aria-hidden="true">›</span>
          </button>
          {expanded && (
            <div className="menu-submenu" role="menu">
              {item.children.map((child, index) =>
                renderItem(child, `${key}-${index}`)
              )}
            </div>
          )}
        </div>
      );
    }
    return (
      <button
        key={key}
        type="button"
        className={`menu-item ${item.danger ? "danger" : ""}`}
        disabled={item.disabled}
        role="menuitem"
        onClick={() => run(item)}
      >
        <span>{item.label}</span>
        {item.shortcut && <span className="menu-shortcut">{item.shortcut}</span>}
      </button>
    );
  };

  return (
    <div className="app-menu-bar" ref={rootRef} role="menubar">
      {menus.map((menu) => {
        const open = openId === menu.id;
        return (
          <div key={menu.id} className="app-menu">
            <button
              type="button"
              className={`app-menu-trigger ${open ? "active" : ""}`}
              aria-haspopup="menu"
              aria-expanded={open}
              onClick={() => {
                setOpenId(open ? null : menu.id);
                setOpenChild(null);
              }}
              // 已经开着时，移到另一个菜单直接切过去。
              onPointerEnter={() => {
                if (openId && openId !== menu.id) {
                  setOpenId(menu.id);
                  setOpenChild(null);
                }
              }}
            >
              {menu.label}
            </button>
            {open && (
              <div className="menu-dropdown" role="menu">
                {menu.items.map((item, index) =>
                  renderItem(item, `${menu.id}-${index}`)
                )}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
