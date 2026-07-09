export async function open(command: string) {
  window.open(command, '_blank', 'noopener,noreferrer')
}

export class Command {
  static create(program: string) {
    return new Command(program)
  }

  constructor(public program: string) {}

  async execute() {
    return {
      code: 1,
      signal: null,
      stdout: '',
      stderr: `服务端 Web 后台不执行本机命令: ${this.program}`,
    }
  }
}
