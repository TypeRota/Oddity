import { describe, expect, it } from "vitest";
import { defineComponent, h, nextTick } from "vue";
import { mount } from "@vue/test-utils";
import HostSlot from "../../src/host/HostSlot.vue";

function page(label: string) {
  return defineComponent({
    setup: () => () => h("section", { class: "page" }, label),
  });
}

describe("HostSlot", () => {
  it("没有贡献时不渲染任何 DOM", () => {
    const wrapper = mount(HostSlot, { props: { contributions: [] } });
    expect(wrapper.html()).toBe("");
    wrapper.unmount();
  });

  it("加载中显示骨架, 不挂载页面", () => {
    const wrapper = mount(HostSlot, {
      props: {
        contributions: [{ id: "page", component: page("页面") }],
        loading: true,
      },
    });
    expect(wrapper.get('[role="status"]').attributes("aria-label")).toBe(
      "正在加载页面",
    );
    expect(wrapper.find(".page").exists()).toBe(false);
    wrapper.unmount();
  });

  it("按传入顺序挂载贡献", () => {
    const wrapper = mount(HostSlot, {
      props: {
        contributions: [
          { id: "update", component: page("更新") },
          { id: "settings", component: page("设置") },
        ],
      },
    });
    expect(wrapper.findAll(".page").map((node) => node.text())).toEqual([
      "更新",
      "设置",
    ]);
    wrapper.unmount();
  });

  it("单个页面崩溃只替换自己, 错误态沿用页面容器语义", async () => {
    const broken = defineComponent({
      setup() {
        throw new Error("页面崩溃");
      },
    });
    const wrapper = mount(HostSlot, {
      props: {
        contributions: [
          { id: "broken", component: broken },
          { id: "ok", component: page("正常页面") },
        ],
      },
    });
    await nextTick();
    const alert = wrapper.get('[role="alert"]');
    expect(alert.attributes("id")).toBe("page-broken");
    expect(alert.classes()).toContain("page");
    expect(alert.attributes("inert")).toBeDefined();
    expect(alert.text()).toContain("页面加载失败");
    expect(alert.text()).toContain("页面崩溃");
    expect(wrapper.get(".page:not([role='alert'])").text()).toBe("正常页面");
    wrapper.unmount();
  });
});
