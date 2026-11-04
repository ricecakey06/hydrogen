// @ts-check
/** @param {import('../../plugin-sdk/launcher').PluginContext} ctx */
export function activate(ctx) {
  ctx.commands.register('show-count', async () => {
    const instances = await ctx.instances.list();
    await ctx.ui.notify({ message: `共有 ${instances.length} 个实例` });
    return { count: instances.length };
  });
}
