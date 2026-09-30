/**
 * 「这一层之上还有没有浮层」——Escape 该归谁。
 *
 * # 为什么必须共用一个判定
 *
 * 每个浮层都自己监听 `keydown`（它们之间没有父子关系，都在 `document` 下平铺）。
 * 谁都不判断的话，一次 Escape 会**同时**关掉所有层：退出确认框盖在设置中心上时
 * 按一下，用户丢的不只是那一次回答，还有刚填了一半的表单。这个判定此前在
 * `settings-center` / `messages-modal` 里各写了一份，列表还不一样（一处漏了
 * `.overlay`）——于是同一个按键在两处的行为不同。现在只有这一份。
 *
 * # 按层档列类名，而不是比对「我自己那个元素」
 *
 * `own` 说的是调用方**属于哪一档**（模态 / 瞬时浮层），不是它的 DOM 元素。
 * 比对元素看起来更精确，但它要求「文档里恰好只有我这一个」——渲染两次就会把
 * 上一次的实例认成「别人」（测试里必然发生），于是 Escape 永远送不出去。
 * 按档列类名没有这个前提：同一档的其它实例本来就该一起让位。
 *
 * # 各档列的是什么
 *
 * - `modal`：设置中心与消息与分支弹框。能压住它们的是 `Dialog` 家族
 *   （新会话 / 重命名 / 复制 / 会话设置 / 粘贴确认，以及**退出确认框**，它随时
 *   可能从后端推来）与贴着按钮的 `.popover`。
 * - `transient`：三个右键菜单与会话提醒弹层。除了上面那些，还包括设置中心与
 *   消息与分支弹框（模态在它们之上）。
 *
 * `.overlay` 是退出确认框从前的自绘外壳类：判定不留着它，R35 那条白名单
 * （按它描述）就形同虚设。
 *
 * # 与 `dialog.tsx` 的实例栈怎么分工
 *
 * 这里回答的是**跨类**的问题：Dialog 之上还压着 `.popover` 吗、模态之上还压着
 * 右键菜单吗。**同类之间**（Dialog 压 Dialog）它答不了——名单只认类名，而调用方
 * 自己也是那一类，于是它看见的是「我自己」。那一路由 `dialog.tsx` 里的实例栈决定：
 * 同类之间只有**挂载顺序**知道谁在最上面，栈顶才响应 Escape。
 */
export type EscapeLayer = "modal" | "transient";

const LAYERS_ABOVE: Record<EscapeLayer, string[]> = {
  modal: [".dialog-overlay", ".overlay", ".popover"],
  transient: [
    ".dialog-overlay",
    ".overlay",
    ".popover",
    ".settings-overlay",
    ".messages-overlay"
  ]
};

/**
 * 有没有比调用方更内层的浮层；有就让出这一次 Escape。
 *
 * # 两个可选参数都在解决「名单里有我自己」
 *
 * - `self`：调用方**自己那一个类名**，从名单里剔掉。`.popover` 同时出现在两档的
 *   名单里（菜单与模态都要注意别被浮层盖住），浮层自己调用时不剔掉就会看见自己，
 *   Escape 再也送不出去。其余调用方（设置中心 / 消息与分支弹框 / 三个右键菜单 /
 *   关注中心 / `Dialog`）的类名本来就不在名单里，不传即可。
 * - `panel`：调用方的面板元素。**包含它**的那一层是它的容器，不是压在它上面的层
 *   ——消息与分支弹框的导出浮层就挂在 `.messages-overlay` **里面**。不跳过容器的话
 *   两者会互相让位，Escape 谁都不响应（浮层关不掉，弹框也关不掉）。
 */
export function hasLayerAbove(
  own: EscapeLayer,
  self?: string,
  panel?: HTMLElement | null
): boolean {
  const selector = LAYERS_ABOVE[own].filter((cls) => cls !== self).join(", ");
  const layers = Array.from(document.querySelectorAll<HTMLElement>(selector));
  if (!panel) return layers.length > 0;
  return layers.some((element) => !element.contains(panel));
}
