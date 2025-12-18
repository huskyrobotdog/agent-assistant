import Database from '@tauri-apps/plugin-sql'

// 数据库单例
let db = null

// 初始化数据库
export const initDb = async () => {
    if (!db) {
        db = await Database.load('sqlite:datas.db')
    }
    return db
}

// 获取数据库实例
export const getDb = () => db
