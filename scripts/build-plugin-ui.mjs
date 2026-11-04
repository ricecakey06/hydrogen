import { build } from 'vite';
import { cp, mkdir, readdir, readFile, writeFile, rm } from 'node:fs/promises';
import path from 'node:path';
await build({configFile:'vite.plugins.config.ts'});
for(const id of ['ui.shell','ui.launcher','plugins.manager']) {
 const target=path.resolve('official-plugins',id,'assets');
 if(!target.startsWith(path.resolve('official-plugins')+path.sep))throw new Error('Invalid package asset path');
 await rm(target,{recursive:true,force:true});await mkdir(target,{recursive:true});
 await cp(path.join('.build/plugin-ui',id,'assets'),target,{recursive:true});
 const file=path.join('official-plugins',id,'plugin.json');const manifest=JSON.parse(await readFile(file,'utf8'));
 manifest.frontend.styles=(await readdir(target)).filter(f=>f.endsWith('.css')).map(f=>'assets/'+f);
 await writeFile(file,JSON.stringify(manifest,null,2)+'\n');
}
