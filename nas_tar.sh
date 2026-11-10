#!/bin/bash
set -e
DEST="/mnt/Disk/downloads/Games/Switch Game/安装包备份"
cd ~/Switch/nsp_share
COPYFILE_DISABLE=1 tar cf - P5R_ASIA.nsp P5R_UPDATE.nsp PokemonArceus_Base.nsp PokemonArceus_Update.nsp GTA_VC_Base.nsp GTA_VC_Update.nsp 0100c2d00ebd4000.nsp 0100c2d00ebd4800.nsp 0100c2d00ebd5001.nsp 0100c2d00ebd5002.nsp 0100c2d00ebd5003.nsp 0100c2d00ebd5004.nsp 0100c2d00ebd5005.nsp | ssh -i ~/.ssh/id_ed25519 root@192.168.100.150 "tar xf - -C '$DEST' && chown -R webdav:webdav '$DEST' && ls -la '$DEST' | wc -l"
echo "NAS_TAR_DONE"
