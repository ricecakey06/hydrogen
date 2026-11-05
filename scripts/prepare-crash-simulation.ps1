param([string]$OutputDirectory, [string]$JavaHome = 'C:\Program Files\Zulu\zulu-17')
$ErrorActionPreference = 'Stop'
$projectRoot = Split-Path $PSScriptRoot -Parent
if (!$OutputDirectory) {
    $OutputDirectory = Join-Path $projectRoot ('artifacts\crash-simulation-' + (Get-Date -Format 'yyyyMMdd-HHmmss'))
}
$simulationRoot = [System.IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $simulationRoot) { throw 'Simulation output must be a new directory.' }
$java = Join-Path $JavaHome 'bin\java.exe'
$javac = Join-Path $JavaHome 'bin\javac.exe'
$jarTool = Join-Path $JavaHome 'bin\jar.exe'
foreach ($tool in @($java,$javac,$jarTool)) { if (!(Test-Path -LiteralPath $tool)) { throw "Missing JDK tool: $tool" } }
New-Item -ItemType Directory -Path $simulationRoot | Out-Null
$classes = Join-Path $simulationRoot 'fixture-classes'
New-Item -ItemType Directory -Path $classes | Out-Null
& $javac -encoding UTF-8 -d $classes (Join-Path $projectRoot 'tests\fixtures\CrashSimulation.java')
if ($LASTEXITCODE -ne 0) { throw 'Failed to compile simulation fixture.' }
$fixtureJar = Join-Path $simulationRoot 'simulation.jar'
& $jarTool --create --file $fixtureJar -C $classes .
if ($LASTEXITCODE -ne 0) { throw 'Failed to package simulation fixture.' }
$jarHash = (Get-FileHash -LiteralPath $fixtureJar -Algorithm SHA1).Hash.ToLowerInvariant()
$jarSize = (Get-Item -LiteralPath $fixtureJar).Length
$data = Join-Path $simulationRoot 'data'
$game = Join-Path $data '.minecraft'
$indexes = Join-Path $game 'assets\indexes'
New-Item -ItemType Directory -Path $indexes -Force | Out-Null
$indexFile = Join-Path $indexes 'simulation.json'
[System.IO.File]::WriteAllText($indexFile, '{"objects":{}}', [System.Text.UTF8Encoding]::new($false))
$indexHash = (Get-FileHash -LiteralPath $indexFile -Algorithm SHA1).Hash.ToLowerInvariant()
$scenarios = @('mixin','heap','metaspace','native-memory','java-version','graphics','dependencies','library','native','unknown','normal','jvm-arguments')
$cases = @()
foreach ($scenario in $scenarios) {
    $name = 'SIM-' + $scenario
    $directory = Join-Path $game ('versions\' + $name)
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
    Copy-Item -LiteralPath $fixtureJar -Destination (Join-Path $directory ($name + '.jar'))
    $jvm = @('-Dfile.encoding=UTF-8','-cp','${classpath}')
    if ($scenario -eq 'jvm-arguments') { $jvm += '-XX:HydrogenSimulationInvalidOption=1' }
    $metadata = [ordered]@{
        id=$name; clientVersion='1.20.1'; type='release'; mainClass='CrashSimulation';
        javaVersion=@{majorVersion=17}; libraries=@();
        assetIndex=@{id='simulation';sha1=$indexHash;size=(Get-Item -LiteralPath $indexFile).Length};
        downloads=@{client=@{sha1=$jarHash;size=$jarSize}};
        arguments=@{jvm=$jvm;game=@($scenario)}
    }
    [System.IO.File]::WriteAllText((Join-Path $directory ($name + '.json')), ($metadata | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
    $sha = [System.Security.Cryptography.SHA1]::Create()
    try { $instanceId = [BitConverter]::ToString($sha.ComputeHash([System.Text.Encoding]::UTF8.GetBytes($game.ToLowerInvariant() + [char]0 + $name))).Replace('-','').ToLowerInvariant().Substring(0,32) }
    finally { $sha.Dispose() }
    $cases += [ordered]@{scenario=$scenario;name=$name;id=$instanceId;directory=$directory}
}
$settings = [ordered]@{
    schemaVersion=1;format='hydrogen-tauri';selectedInstanceId=$cases[0].id;
    settings=[ordered]@{
        gameDirectory=$null;isolateGameDirectory=$true;java=$java;maxMemoryMib=512;
        windowMode='default';windowWidth=854;windowHeight=480;afterLaunch='keep';jvmArguments='';
        downloadSource='official';offlineEnabled=$true;offlinePlayerName='CrashTester';accountMode='offline';
        microsoftClientId='';modules=@{}
    }
}
[System.IO.File]::WriteAllText((Join-Path $data 'settings.json'), ($settings | ConvertTo-Json -Depth 12), [System.Text.UTF8Encoding]::new($false))
New-Item -ItemType Directory -Path (Join-Path $simulationRoot 'exports') | Out-Null
$manifest = [ordered]@{root=$simulationRoot;data=$data;game=$game;java=$java;cases=$cases;createdAt=(Get-Date).ToString('o')}
$manifest | ConvertTo-Json -Depth 12 | Set-Content -LiteralPath (Join-Path $simulationRoot 'simulation.json') -Encoding UTF8
$manifest | ConvertTo-Json -Depth 12
