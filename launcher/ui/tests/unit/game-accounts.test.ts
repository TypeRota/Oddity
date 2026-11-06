import { mount, flushPromises, type VueWrapper } from "@vue/test-utils";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
const api = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("../../src/core/api", () => ({
  invoke: api.invoke,
  isDesktop: () => false,
}));
import Dialog from "../../src/components/GameAccountDialog.vue";
let wrapper: VueWrapper;
beforeEach(() => {
  api.invoke
    .mockReset()
    .mockImplementation(async (command) =>
      command === "game_accounts" ? [] : "b".repeat(32),
    );
});
afterEach(() => {
  wrapper?.unmount();
});
async function open(selected = "") {
  wrapper = mount(Dialog, {
    props: { open: false, selected, theme: "light", reduceMotion: true },
    global: {
      stubs: {
        BaseModal: {
          props: ["open"],
          template: '<div v-if="open"><slot /></div>',
        },
      },
    },
  });
  await wrapper.setProps({ open: true });
  await flushPromises();
}
async function click(text: string) {
  const button = wrapper
    .findAll("button")
    .find((button) => button.text() === text);
  expect(button).toBeTruthy();
  await button!.trigger("click");
  await flushPromises();
}
it("选择保存账号仅返回内部编号, 不发短信或重新登录", async () => {
  api.invoke.mockResolvedValue([
    { accountId: "a".repeat(32), maskedPhone: "138****8000" },
  ]);
  await open();
  expect(wrapper.text()).toContain("138****8000");
  await click("选择账号");
  expect(wrapper.emitted("select")).toEqual([["a".repeat(32)]]);
  expect(api.invoke).toHaveBeenCalledTimes(1);
});
it("移除账号先确认, 取消后保留账号和当前选择", async () => {
  const id = "a".repeat(32);
  api.invoke.mockResolvedValue([{ accountId: id, maskedPhone: "138****8000" }]);
  await open(id);
  const remove = wrapper
    .findAll("button")
    .find((button) => button.text() === "移除")!;
  expect(remove.classes()).toContain("control-button--danger");
  await click("移除");
  expect(wrapper.text()).toContain("确认移除此账号?");
  expect(api.invoke).not.toHaveBeenCalledWith(
    "game_account_remove",
    expect.anything(),
  );
  await click("取消");
  expect(wrapper.text()).not.toContain("确认移除此账号?");
  expect(wrapper.text()).toContain("138****8000");
  expect(wrapper.emitted("select")).toBeUndefined();
  expect(api.invoke).toHaveBeenCalledTimes(1);
});
it("确认移除仅删除目标账号, 成功后清空被删除账号的选择", async () => {
  const id = "a".repeat(32);
  api.invoke.mockResolvedValueOnce([
    { accountId: id, maskedPhone: "138****8000" },
    { accountId: "b".repeat(32), maskedPhone: "139****9000" },
  ]);
  await open(id);
  await wrapper
    .findAll(".game-accounts__row")[0]
    .findAll("button")
    .at(-1)!
    .trigger("click");
  await flushPromises();
  await click("确认移除");
  expect(api.invoke).toHaveBeenCalledWith("game_account_remove", {
    accountId: id,
  });
  expect(wrapper.text()).not.toContain("138****8000");
  expect(wrapper.text()).toContain("139****9000");
  expect(wrapper.emitted("select")).toEqual([[""]]);
  expect(wrapper.text()).not.toContain("确认移除此账号?");
});
it("移除失败时保留账号和选择, 允许取消确认", async () => {
  const id = "a".repeat(32);
  api.invoke.mockResolvedValueOnce([
    { accountId: id, maskedPhone: "138****8000" },
  ]);
  await open(id);
  await click("移除");
  api.invoke.mockRejectedValueOnce(new Error("密钥环暂时不可用."));
  await click("确认移除");
  expect(wrapper.get('[role="alert"]').text()).toBe("密钥环暂时不可用.");
  expect(wrapper.text()).toContain("138****8000");
  expect(wrapper.emitted("select")).toBeUndefined();
  await click("取消");
  expect(wrapper.text()).not.toContain("确认移除此账号?");
});
it("关闭后重新打开或进入添加表单时清除移除确认", async () => {
  api.invoke.mockResolvedValue([
    { accountId: "a".repeat(32), maskedPhone: "138****8000" },
  ]);
  await open();
  await click("移除");
  await wrapper.setProps({ open: false });
  await wrapper.setProps({ open: true });
  await flushPromises();
  expect(wrapper.text()).not.toContain("确认移除此账号?");
  await click("移除");
  await click("添加游戏账号");
  await click("返回账号列表");
  expect(wrapper.text()).not.toContain("确认移除此账号?");
  expect(
    api.invoke.mock.calls.some(([name]) => name === "game_account_remove"),
  ).toBe(false);
});
it("短信验证码绑定取码时的手机号, 修改号码后不能直接提交", async () => {
  await open();
  await wrapper.get('input[type="tel"]').setValue("13800138000");
  await click("获取验证码");
  await wrapper.get('input[type="tel"]').setValue("13900139000");
  await wrapper.get('input[autocomplete="one-time-code"]').setValue("123456");
  await wrapper.get("form").trigger("submit");
  await flushPromises();
  expect(wrapper.text()).toContain("请先为当前手机号获取验证码");
  expect(api.invoke.mock.calls.some(([name]) => name === "game_login")).toBe(
    false,
  );
});
it("服务端要求短信时切换表单并清空密码, 不自动发送短信", async () => {
  await open();
  await click("密码");
  await wrapper.get('input[type="tel"]').setValue("13800138000");
  await wrapper.get('input[type="password"]').setValue("synthetic-password");
  api.invoke.mockRejectedValueOnce(
    new Error("为保障账号安全, 服务端要求使用短信验证码登录."),
  );
  await wrapper.get("form").trigger("submit");
  await flushPromises();
  expect(wrapper.find('input[type="password"]').exists()).toBe(false);
  expect(
    wrapper.get('input[autocomplete="one-time-code"]').element,
  ).toHaveProperty("value", "");
  expect(wrapper.text()).toContain("服务端要求使用短信验证码登录");
  expect(
    api.invoke.mock.calls.filter(([name]) => name === "game_send_code"),
  ).toHaveLength(0);
  expect(wrapper.emitted("select")).toBeUndefined();
});
it("成功登录后选择保存账号, 关闭时清空验证码", async () => {
  await open();
  await wrapper.get('input[type="tel"]').setValue("13800138000");
  await click("获取验证码");
  await wrapper.get('input[autocomplete="one-time-code"]').setValue("123456");
  api.invoke.mockResolvedValueOnce({
    accountId: "a".repeat(32),
    maskedPhone: "138****8000",
  });
  await wrapper.get("form").trigger("submit");
  await flushPromises();
  expect(wrapper.emitted("select")).toEqual([["a".repeat(32)]]);
  expect(wrapper.emitted("update:open")).toEqual([[false]]);
  expect(
    wrapper.get('input[autocomplete="one-time-code"]').element,
  ).toHaveProperty("value", "");
});
